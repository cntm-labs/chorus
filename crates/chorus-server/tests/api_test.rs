use async_trait::async_trait;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

// Re-use types from chorus-server
use chorus_server::app::{create_router, AppState};
use chorus_server::config::Config;
use chorus_server::db::{
    Account, AccountRepository, ApiKey, ApiKeyRepository, DbError, DeliveryEvent, Message,
    MessageRepository, NewMessage, NewProviderConfig, NewWebhook, Pagination, ProviderConfig,
    ProviderConfigRepository, Webhook, WebhookRepository,
};

// ---------------------------------------------------------------------------
// Mock repositories
// ---------------------------------------------------------------------------

struct MockAccountRepo {
    account: Account,
    api_key: ApiKey,
    key_hash: String,
}

#[async_trait]
impl AccountRepository for MockAccountRepo {
    async fn find_by_api_key_hash(&self, hash: &str) -> Result<Option<(Account, ApiKey)>, DbError> {
        if hash == self.key_hash {
            Ok(Some((self.account.clone(), self.api_key.clone())))
        } else {
            Ok(None)
        }
    }

    async fn update_key_last_used(&self, _key_id: Uuid) -> Result<(), DbError> {
        Ok(())
    }
}

struct MockMessageRepo {
    messages: Mutex<Vec<Message>>,
}

impl MockMessageRepo {
    fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl MessageRepository for MockMessageRepo {
    async fn insert(&self, msg: &NewMessage) -> Result<Message, DbError> {
        let message = Message {
            id: Uuid::new_v4(),
            account_id: msg.account_id,
            api_key_id: msg.api_key_id,
            channel: msg.channel.clone(),
            provider: None,
            sender: msg.sender.clone(),
            recipient: msg.recipient.clone(),
            subject: msg.subject.clone(),
            body: msg.body.clone(),
            status: "queued".into(),
            provider_message_id: None,
            error_message: None,
            cost_microdollars: 0,
            attempts: 0,
            environment: msg.environment.clone(),
            created_at: Utc::now(),
            delivered_at: None,
        };
        self.messages.lock().unwrap().push(message.clone());
        Ok(message)
    }

    async fn find_by_id(&self, id: Uuid, account_id: Uuid) -> Result<Option<Message>, DbError> {
        let msgs = self.messages.lock().unwrap();
        Ok(msgs
            .iter()
            .find(|m| m.id == id && m.account_id == account_id)
            .cloned())
    }

    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &Pagination,
    ) -> Result<Vec<Message>, DbError> {
        let msgs = self.messages.lock().unwrap();
        let filtered: Vec<_> = msgs
            .iter()
            .filter(|m| m.account_id == account_id)
            .skip(pagination.offset as usize)
            .take(pagination.limit as usize)
            .cloned()
            .collect();
        Ok(filtered)
    }

    async fn update_status(
        &self,
        _id: Uuid,
        _status: &str,
        _provider: Option<&str>,
        _provider_message_id: Option<&str>,
        _error_message: Option<&str>,
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn insert_delivery_event(
        &self,
        _message_id: Uuid,
        _status: &str,
        _provider_data: Option<serde_json::Value>,
    ) -> Result<(), DbError> {
        Ok(())
    }

    async fn get_delivery_events(&self, _message_id: Uuid) -> Result<Vec<DeliveryEvent>, DbError> {
        Ok(vec![])
    }
}

struct MockApiKeyRepo;

#[async_trait]
impl ApiKeyRepository for MockApiKeyRepo {
    async fn list_by_account(&self, _account_id: Uuid) -> Result<Vec<ApiKey>, DbError> {
        Ok(vec![])
    }

    async fn insert(
        &self,
        account_id: Uuid,
        name: &str,
        _key_hash: &str,
        key_prefix: &str,
        environment: &str,
    ) -> Result<ApiKey, DbError> {
        Ok(ApiKey {
            id: Uuid::new_v4(),
            account_id,
            name: name.into(),
            key_prefix: key_prefix.into(),
            environment: environment.into(),
            last_used_at: None,
            expires_at: None,
            is_revoked: false,
            created_at: Utc::now(),
        })
    }

    async fn revoke(&self, _id: Uuid, _account_id: Uuid) -> Result<(), DbError> {
        Ok(())
    }
}

struct MockWebhookRepo;

#[async_trait]
impl WebhookRepository for MockWebhookRepo {
    async fn insert(&self, webhook: &NewWebhook) -> Result<Webhook, DbError> {
        Ok(Webhook {
            id: Uuid::new_v4(),
            account_id: webhook.account_id,
            url: webhook.url.clone(),
            secret: webhook.secret.clone(),
            events: webhook.events.clone(),
            is_active: true,
            created_at: Utc::now(),
        })
    }

    async fn list_by_account(&self, _account_id: Uuid) -> Result<Vec<Webhook>, DbError> {
        Ok(vec![])
    }

    async fn list_by_account_event(
        &self,
        _account_id: Uuid,
        _event: &str,
    ) -> Result<Vec<Webhook>, DbError> {
        Ok(vec![])
    }

    async fn delete(&self, _id: Uuid, _account_id: Uuid) -> Result<(), DbError> {
        Ok(())
    }
}

struct MockProviderConfigRepo;

#[async_trait]
impl ProviderConfigRepository for MockProviderConfigRepo {
    async fn list_by_account_channel(
        &self,
        _account_id: Uuid,
        _channel: &str,
    ) -> Result<Vec<ProviderConfig>, DbError> {
        Ok(vec![])
    }

    async fn insert(&self, config: &NewProviderConfig) -> Result<ProviderConfig, DbError> {
        Ok(ProviderConfig {
            id: Uuid::new_v4(),
            account_id: config.account_id,
            channel: config.channel.clone(),
            provider: config.provider.clone(),
            priority: config.priority,
            credentials: config.credentials.clone(),
            is_active: true,
            created_at: Utc::now(),
        })
    }

    async fn list_by_account(&self, _account_id: Uuid) -> Result<Vec<ProviderConfig>, DbError> {
        Ok(vec![])
    }

    async fn delete(&self, _id: Uuid, _account_id: Uuid) -> Result<(), DbError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

const TEST_API_KEY: &str =
    "ch_test_abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

fn test_state() -> Arc<AppState> {
    let key_hash = hex::encode(Sha256::digest(TEST_API_KEY.as_bytes()));
    let account_id = Uuid::new_v4();
    let key_id = Uuid::new_v4();

    let account_repo = Arc::new(MockAccountRepo {
        account: Account {
            id: account_id,
            name: "Test Account".into(),
            owner_email: "test@example.com".into(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        api_key: ApiKey {
            id: key_id,
            account_id,
            name: "test key".into(),
            key_prefix: "ch_test_abcdef12...".into(),
            environment: "test".into(),
            last_used_at: None,
            expires_at: None,
            is_revoked: false,
            created_at: Utc::now(),
        },
        key_hash,
    });

    let message_repo = Arc::new(MockMessageRepo::new());
    let api_key_repo = Arc::new(MockApiKeyRepo);
    let provider_config_repo = Arc::new(MockProviderConfigRepo);
    let webhook_repo = Arc::new(MockWebhookRepo);

    // Use a dummy Redis URL — tests that hit Redis will fail,
    // but auth + DB-only tests will work
    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();

    let config = Arc::new(Config::from_env());
    Arc::new(AppState::with_repos(
        redis,
        config,
        account_repo,
        message_repo,
        api_key_repo,
        provider_config_repo,
        webhook_repo,
    ))
}

async fn response_body(resp: axum::response::Response) -> Value {
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sms_send_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"to":"+1234567890","body":"hello"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sms_send_with_invalid_key_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("content-type", "application/json")
                .header("authorization", "Bearer ch_test_invalid")
                .body(axum::body::Body::from(
                    r#"{"to":"+1234567890","body":"hello"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sms_send_with_bad_format_key_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("content-type", "application/json")
                .header("authorization", "Bearer sk_not_a_chorus_key")
                .body(axum::body::Body::from(
                    r#"{"to":"+1234567890","body":"hello"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn messages_list_returns_empty_for_new_account() {
    let state = test_state();
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn keys_list_returns_200() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/keys")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn providers_list_returns_200() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/providers")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert!(body.as_array().unwrap().is_empty());
}
