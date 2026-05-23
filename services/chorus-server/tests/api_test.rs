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
use chorus_server::crypto::Encryptor;
use chorus_server::db::{
    Account, AccountRepository, AddSuppressionResult, ApiKey, ApiKeyRepository, DbError,
    DeliveryEvent, IdempotencyOutcome, IdempotencyRepository, Message, MessageRepository,
    NewMessage, NewProviderConfig, NewSuppression, NewTotpUser, NewVerification, NewWebhook,
    Pagination, ProviderConfig, ProviderConfigRepository, Suppression, SuppressionRepository,
    TotpRepository, TotpUser, Verification, VerificationRepository, Webhook, WebhookRepository,
};

// ---------------------------------------------------------------------------
// Mock repositories
// ---------------------------------------------------------------------------

/// No-op idempotency repo: returns Fresh for every call so existing tests
/// behave as if the header was absent. Used by the default test fixture.
struct NullIdempotencyRepo;

#[async_trait]
impl IdempotencyRepository for NullIdempotencyRepo {
    async fn begin(
        &self,
        _api_key_id: Uuid,
        _key: &str,
        _request_hash: &[u8; 32],
        _method: &str,
        _path: &str,
    ) -> Result<IdempotencyOutcome, DbError> {
        Ok(IdempotencyOutcome::Fresh)
    }
    async fn complete(
        &self,
        _api_key_id: Uuid,
        _key: &str,
        _response_status: u16,
        _response_body: &[u8],
    ) -> Result<(), DbError> {
        Ok(())
    }
    async fn delete_expired(&self, _limit: i64) -> Result<u64, DbError> {
        Ok(0)
    }
}

/// No-op verification repo for tests that don't exercise verification logic.
struct NullVerificationRepo;

#[async_trait]
impl chorus_server::db::VerificationRepository for NullVerificationRepo {
    async fn insert(
        &self,
        _v: &chorus_server::db::NewVerification,
    ) -> Result<chorus_server::db::Verification, DbError> {
        Err(DbError::Internal(anyhow::anyhow!(
            "NullVerificationRepo::insert not implemented"
        )))
    }
    async fn find_by_id(
        &self,
        _id: Uuid,
        _account_id: Uuid,
    ) -> Result<Option<chorus_server::db::Verification>, DbError> {
        Ok(None)
    }
    async fn list_by_account(
        &self,
        _account_id: Uuid,
        _pagination: &chorus_server::db::Pagination,
    ) -> Result<Vec<chorus_server::db::Verification>, DbError> {
        Ok(vec![])
    }
    async fn increment_check_attempts(&self, _id: Uuid, _account_id: Uuid) -> Result<i32, DbError> {
        Err(DbError::NotFound)
    }
    async fn mark_approved(&self, _id: Uuid, _account_id: Uuid) -> Result<(), DbError> {
        Err(DbError::NotFound)
    }
    async fn mark_canceled(&self, _id: Uuid, _account_id: Uuid) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn record_resend(
        &self,
        _id: Uuid,
        _account_id: Uuid,
        _cost: i64,
        _max: i32,
    ) -> Result<chorus_server::db::Verification, DbError> {
        Err(DbError::NotFound)
    }
    async fn expire_pending(&self, _limit: i64) -> Result<u64, DbError> {
        Ok(0)
    }
}

/// No-op TOTP repo for tests that don't exercise TOTP logic.
struct NullTotpRepo;

#[async_trait]
impl TotpRepository for NullTotpRepo {
    async fn enroll(
        &self,
        _new_user: &NewTotpUser,
        _hashes: &[Vec<u8>],
    ) -> Result<TotpUser, DbError> {
        Err(DbError::Internal(anyhow::anyhow!("NullTotpRepo::enroll not implemented")))
    }
    async fn find(&self, _account_id: Uuid, _user_id: &str) -> Result<Option<TotpUser>, DbError> {
        Ok(None)
    }
    async fn activate(&self, _account_id: Uuid, _user_id: &str) -> Result<(), DbError> {
        Err(DbError::NotFound)
    }
    async fn touch_last_verified(
        &self,
        _account_id: Uuid,
        _user_id: &str,
    ) -> Result<(), DbError> {
        Ok(())
    }
    async fn disenroll(&self, _account_id: Uuid, _user_id: &str) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn consume_backup_code(
        &self,
        _account_id: Uuid,
        _user_id: &str,
        _hash: &[u8],
    ) -> Result<bool, DbError> {
        Ok(false)
    }
    async fn unused_backup_codes_count(
        &self,
        _account_id: Uuid,
        _user_id: &str,
    ) -> Result<i64, DbError> {
        Ok(0)
    }
    async fn replace_backup_codes(
        &self,
        _account_id: Uuid,
        _user_id: &str,
        _hashes: &[Vec<u8>],
    ) -> Result<(), DbError> {
        Ok(())
    }
}

/// Test helper: 32-byte zero-key Encryptor (NOT for production).
fn null_encryptor() -> Arc<Encryptor> {
    use base64::Engine;
    let key_b64 = base64::engine::general_purpose::STANDARD.encode([0u8; 32]);
    let prev = std::env::var("CHORUS_ENCRYPTION_KEY").ok();
    std::env::set_var("CHORUS_ENCRYPTION_KEY", &key_b64);
    let enc = Encryptor::from_env().expect("encryptor from null key");
    match prev {
        Some(v) => std::env::set_var("CHORUS_ENCRYPTION_KEY", v),
        None => std::env::remove_var("CHORUS_ENCRYPTION_KEY"),
    }
    Arc::new(enc)
}

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
    delivery_events: Mutex<Vec<DeliveryEvent>>,
}

impl MockMessageRepo {
    fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            delivery_events: Mutex::new(Vec::new()),
        }
    }

    /// Directly insert a pre-built message without going through the HTTP layer — test helper.
    fn seed(&self, msg: Message) {
        self.messages.lock().unwrap().push(msg);
    }

    /// Read a snapshot of all delivery events — test helper.
    fn delivery_events_snapshot(&self) -> Vec<DeliveryEvent> {
        self.delivery_events.lock().unwrap().clone()
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
        id: Uuid,
        status: &str,
        provider: Option<&str>,
        provider_message_id: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), DbError> {
        let mut msgs = self.messages.lock().unwrap();
        if let Some(m) = msgs.iter_mut().find(|m| m.id == id) {
            m.status = status.to_string();
            if let Some(p) = provider {
                m.provider = Some(p.to_string());
            }
            if let Some(pmid) = provider_message_id {
                m.provider_message_id = Some(pmid.to_string());
            }
            if let Some(err) = error_message {
                m.error_message = Some(err.to_string());
            }
        }
        Ok(())
    }

    async fn insert_delivery_event(
        &self,
        message_id: Uuid,
        status: &str,
        provider_data: Option<serde_json::Value>,
    ) -> Result<(), DbError> {
        self.delivery_events.lock().unwrap().push(DeliveryEvent {
            id: Uuid::new_v4(),
            message_id,
            status: status.to_string(),
            provider_data,
            created_at: Utc::now(),
        });
        Ok(())
    }

    async fn get_delivery_events(&self, message_id: Uuid) -> Result<Vec<DeliveryEvent>, DbError> {
        let events = self.delivery_events.lock().unwrap();
        Ok(events
            .iter()
            .filter(|e| e.message_id == message_id)
            .cloned()
            .collect())
    }

    async fn find_by_provider_message_id(
        &self,
        provider_message_id: &str,
    ) -> Result<Option<Message>, DbError> {
        let msgs = self.messages.lock().unwrap();
        Ok(msgs
            .iter()
            .find(|m| m.provider_message_id.as_deref() == Some(provider_message_id))
            .cloned())
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

struct MockSuppressionRepo {
    entries: Mutex<Vec<Suppression>>,
}

impl MockSuppressionRepo {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    fn snapshot(&self) -> Vec<Suppression> {
        self.entries.lock().unwrap().clone()
    }
}

#[async_trait]
impl SuppressionRepository for MockSuppressionRepo {
    async fn is_suppressed(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<Option<String>, DbError> {
        let entries = self.entries.lock().unwrap();
        Ok(entries
            .iter()
            .find(|e| {
                e.account_id == account_id && e.channel == channel && e.recipient == recipient
            })
            .map(|e| e.reason.clone()))
    }

    async fn add(&self, entry: &NewSuppression) -> Result<AddSuppressionResult, DbError> {
        let mut entries = self.entries.lock().unwrap();
        if let Some(existing) = entries.iter().find(|e| {
            e.account_id == entry.account_id
                && e.channel == entry.channel
                && e.recipient == entry.recipient
        }) {
            return Ok(AddSuppressionResult {
                entry: existing.clone(),
                inserted: false,
            });
        }
        let row = Suppression {
            account_id: entry.account_id,
            channel: entry.channel.clone(),
            recipient: entry.recipient.clone(),
            reason: entry.reason.clone(),
            source: entry.source.clone(),
            created_at: Utc::now(),
        };
        entries.push(row.clone());
        Ok(AddSuppressionResult {
            entry: row,
            inserted: true,
        })
    }

    async fn remove(
        &self,
        account_id: Uuid,
        channel: &str,
        recipient: &str,
    ) -> Result<bool, DbError> {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries.retain(|e| {
            !(e.account_id == account_id && e.channel == channel && e.recipient == recipient)
        });
        Ok(entries.len() < before)
    }

    async fn list(
        &self,
        account_id: Uuid,
        channel: Option<&str>,
        pagination: &Pagination,
    ) -> Result<Vec<Suppression>, DbError> {
        let entries = self.entries.lock().unwrap();
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| e.account_id == account_id)
            .filter(|e| channel.is_none_or(|c| e.channel == c))
            .skip(pagination.offset as usize)
            .take(pagination.limit as usize)
            .cloned()
            .collect();
        Ok(filtered)
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

struct TestFixture {
    state: Arc<AppState>,
    suppressions: Arc<MockSuppressionRepo>,
    messages: Arc<MockMessageRepo>,
    account_id: Uuid,
}

fn test_fixture() -> TestFixture {
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

    let messages = Arc::new(MockMessageRepo::new());
    let suppressions = Arc::new(MockSuppressionRepo::new());
    let api_key_repo = Arc::new(MockApiKeyRepo);
    let provider_config_repo = Arc::new(MockProviderConfigRepo);
    let webhook_repo = Arc::new(MockWebhookRepo);

    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let config = Arc::new(Config::from_env());

    let idempotency_repo: Arc<dyn IdempotencyRepository> = Arc::new(NullIdempotencyRepo);
    let verification_repo: Arc<dyn VerificationRepository> = Arc::new(NullVerificationRepo);
    let totp_repo: Arc<dyn TotpRepository> = Arc::new(NullTotpRepo);
    let encryptor = null_encryptor();

    let state = Arc::new(AppState::with_repos(
        redis,
        config,
        account_repo,
        messages.clone(),
        api_key_repo,
        provider_config_repo,
        webhook_repo,
        suppressions.clone(),
        idempotency_repo,
        verification_repo,
        totp_repo,
        encryptor,
    ));

    TestFixture {
        state,
        suppressions,
        messages,
        account_id,
    }
}

fn test_state() -> Arc<AppState> {
    test_fixture().state
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

// ---------------------------------------------------------------------------
// Batch SMS tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sms_batch_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send-batch")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipients":[{"to":"+1234567890","body":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sms_batch_empty_recipients_returns_400() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send-batch")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::from(r#"{"recipients":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sms_batch_exceeds_max_returns_400() {
    let app = create_router(test_state());

    // Build 101 recipients (max is 100)
    let recipients: Vec<Value> = (0..101)
        .map(|i| {
            serde_json::json!({
                "to": format!("+1{:010}", i),
                "body": "hello"
            })
        })
        .collect();
    let body = serde_json::json!({ "recipients": recipients }).to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send-batch")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Batch Email tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn email_batch_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send-batch")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipients":[{"to":"a@b.com","subject":"hi","body":"hey"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn email_batch_empty_recipients_returns_400() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send-batch")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::from(r#"{"recipients":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn email_batch_exceeds_max_returns_400() {
    let app = create_router(test_state());

    let recipients: Vec<Value> = (0..101)
        .map(|i| {
            serde_json::json!({
                "to": format!("user{}@example.com", i),
                "subject": "test",
                "body": "hello"
            })
        })
        .collect();
    let body = serde_json::json!({ "recipients": recipients }).to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send-batch")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Webhook tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_create_returns_201_with_secret() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/webhooks")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::from(
                    r#"{"url":"https://example.com/hook","events":["message.delivered"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = response_body(resp).await;
    assert!(body["id"].is_string());
    assert_eq!(body["url"], "https://example.com/hook");
    assert!(body["secret"].as_str().unwrap().len() >= 32);
    assert_eq!(body["events"][0], "message.delivered");
}

#[tokio::test]
async fn webhook_create_invalid_event_returns_400() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/webhooks")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::from(
                    r#"{"url":"https://example.com/hook","events":["invalid.event"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn webhook_list_returns_200() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/webhooks")
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

#[tokio::test]
async fn webhook_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/webhooks")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"url":"https://example.com/hook","events":["message.delivered"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Admin auth tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_accounts_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/accounts")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_accounts_with_user_key_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/accounts")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // User API keys (ch_test_) should be rejected by admin endpoint
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_accounts_with_invalid_admin_key_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/accounts")
                .header("authorization", "Bearer ch_admin_invalid_key")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // NullAdminKeyRepository returns None → 401
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_providers_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/providers")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_messages_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/messages")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_billing_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/billing/accounts")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_webhooks_without_auth_returns_401() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/webhooks")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Billing tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn billing_plans_returns_200() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/billing/plans")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert!(body["plans"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn billing_usage_returns_200() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/billing/usage")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn billing_checkout_without_stripe_returns_503() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/billing/checkout")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::from(
                    r#"{"plan_slug":"starter","success_url":"https://example.com/ok","cancel_url":"https://example.com/cancel"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // No STRIPE_SECRET_KEY configured → 503
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------------
// Suppression list tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_suppressions_empty_returns_200() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    assert_eq!(body["data"], serde_json::json!([]));
    assert_eq!(body["limit"], 20);
    assert_eq!(body["offset"], 0);
}

#[tokio::test]
async fn create_suppression_normalizes_email_and_returns_201() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"  Alice@Example.COM "}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = response_body(resp).await;
    assert_eq!(body["recipient"], "alice@example.com");
    assert_eq!(body["reason"], "manual");
    assert_eq!(body["source"], "api");
}

#[tokio::test]
async fn create_suppression_rejects_bad_e164() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"sms","recipient":"0812345678"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_suppression_round_trip() {
    let state = test_state();
    let app = create_router(Arc::clone(&state));

    // Add
    let add = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"bob@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::CREATED);

    // Delete
    let del = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/suppressions/email/bob@example.com")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    // Delete again → 404
    let del2 = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/suppressions/email/bob@example.com")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del2.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sms_send_to_suppressed_recipient_returns_422() {
    let state = test_state();
    let app = create_router(Arc::clone(&state));

    // Pre-populate suppression
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"sms","recipient":"+14155552671"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"to":"+14155552671","body":"hi"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_body(resp).await;
    assert_eq!(body["error"]["code"], "recipient_suppressed");
    assert_eq!(body["error"]["reason"], "manual");
}

#[tokio::test]
async fn email_send_to_suppressed_recipient_returns_422() {
    let app = create_router(test_state());

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"alice@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"to":"ALICE@example.com","subject":"hi","body":"hi"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_body(resp).await;
    assert_eq!(body["error"]["code"], "recipient_suppressed");
}

#[tokio::test]
async fn otp_send_to_suppressed_email_returns_422() {
    let app = create_router(test_state());

    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"otp@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/otp/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(r#"{"to":"otp@example.com"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn email_batch_with_suppressed_recipient_returns_207() {
    let app = create_router(test_state());

    // Suppress one recipient
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"bad@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send-batch")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipients":[
                        {"to":"good@example.com","subject":"x","body":"y"},
                        {"to":"bad@example.com","subject":"x","body":"y"}
                    ]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::MULTI_STATUS);
    let body = response_body(resp).await;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    let suppressed: Vec<_> = messages
        .iter()
        .filter(|m| m["status"] == "suppressed")
        .collect();
    assert_eq!(suppressed.len(), 1);
    assert_eq!(suppressed[0]["to"], "bad@example.com");
    assert_eq!(suppressed[0]["reason"], "manual");
}

#[tokio::test]
async fn bounce_creates_suppression_and_marks_message_bounced() {
    std::env::set_var("BOUNCE_SECRET", "test-secret");

    let fx = test_fixture();
    let app = create_router(Arc::clone(&fx.state));

    // Seed a message row directly (bypassing HTTP/Redis so the test stays self-contained).
    let seeded_id = Uuid::new_v4();
    fx.messages.seed(Message {
        id: seeded_id,
        account_id: fx.account_id,
        api_key_id: Uuid::new_v4(),
        channel: "email".into(),
        provider: None,
        sender: None,
        recipient: "bouncy@example.com".into(),
        subject: Some("x".into()),
        body: "y".into(),
        status: "queued".into(),
        provider_message_id: Some("bounce-test-1".into()),
        error_message: None,
        cost_microdollars: 0,
        attempts: 0,
        environment: "test".into(),
        created_at: Utc::now(),
        delivered_at: None,
    });

    // POST the bounce.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/bounces")
                .header("x-chorus-secret", "test-secret")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipient":"bouncy@example.com","reason":"5.1.1 user unknown","message_id":"bounce-test-1"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify suppression created.
    let snapshot = fx.suppressions.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].channel, "email");
    assert_eq!(snapshot[0].recipient, "bouncy@example.com");
    assert_eq!(snapshot[0].reason, "hard_bounce");
    assert_eq!(snapshot[0].source, "chorus-mail");
    assert_eq!(snapshot[0].account_id, fx.account_id);

    // Verify message status flipped to "bounced".
    let updated = fx
        .messages
        .find_by_id(seeded_id, fx.account_id)
        .await
        .unwrap()
        .expect("seeded message should still exist");
    assert_eq!(updated.status, "bounced");
    assert_eq!(updated.error_message.as_deref(), Some("5.1.1 user unknown"));

    // Verify a delivery_event row was appended with status="bounced".
    let events = fx.messages.delivery_events_snapshot();
    let bounced_events: Vec<_> = events
        .iter()
        .filter(|e| e.message_id == seeded_id && e.status == "bounced")
        .collect();
    assert_eq!(bounced_events.len(), 1);
    let provider_data = bounced_events[0].provider_data.as_ref().unwrap();
    assert_eq!(provider_data["reason"], "5.1.1 user unknown");
    assert_eq!(provider_data["source"], "chorus-mail");
}

#[tokio::test]
async fn bounce_with_unknown_message_id_returns_200_no_suppression() {
    std::env::set_var("BOUNCE_SECRET", "test-secret");

    let fx = test_fixture();
    let app = create_router(Arc::clone(&fx.state));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/internal/bounces")
                .header("x-chorus-secret", "test-secret")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipient":"x@example.com","reason":"5.1.1","message_id":"never-existed"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(fx.suppressions.snapshot().is_empty());
}

#[tokio::test]
async fn create_suppression_idempotent_returns_200_on_duplicate() {
    let app = create_router(test_state());

    // First call → 201 Created
    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"dup@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let first_body = response_body(first).await;
    let first_created_at = first_body["created_at"].as_str().unwrap().to_string();

    // Second call → 200 OK (idempotent), same created_at echoed back
    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"dup@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second_body = response_body(second).await;
    assert_eq!(second_body["created_at"], first_created_at);
    assert_eq!(second_body["reason"], "manual");
}

#[tokio::test]
async fn create_suppression_forces_server_side_reason_and_source() {
    // Even if a client sends extra fields like `reason`/`source`, the server
    // must override them. CreateSuppressionRequest only deserializes
    // `channel` and `recipient`, so unknown fields are silently dropped —
    // but this test pins the contract.
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"forced@example.com","reason":"hard_bounce","source":"chorus-mail"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = response_body(resp).await;
    assert_eq!(body["reason"], "manual");
    assert_eq!(body["source"], "api");
}

#[tokio::test]
async fn list_suppressions_filters_by_channel() {
    let app = create_router(test_state());

    // Seed one email + one sms suppression.
    for payload in [
        r#"{"channel":"email","recipient":"e@example.com"}"#,
        r#"{"channel":"sms","recipient":"+14155552671"}"#,
    ] {
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/suppressions")
                    .header("authorization", format!("Bearer {TEST_API_KEY}"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/suppressions?channel=email")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = response_body(resp).await;
    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["channel"], "email");
    assert_eq!(data[0]["recipient"], "e@example.com");
}

#[tokio::test]
async fn sms_batch_with_invalid_e164_marks_entry_invalid_and_continues() {
    let app = create_router(test_state());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send-batch")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"recipients":[
                        {"to":"+14155552671","body":"hi"},
                        {"to":"0812345678","body":"bad e164"}
                    ]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::MULTI_STATUS);
    let body = response_body(resp).await;
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    let invalid: Vec<_> = messages
        .iter()
        .filter(|m| m["status"] == "invalid")
        .collect();
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0]["to"], "0812345678");
    assert!(invalid[0]["reason"].as_str().unwrap().contains("E.164"));
}

// ----- C1 idempotency tests -----

/// One row of the in-memory idempotency table:
/// (request_hash, status, response_status, response_body).
type MemIdempotencyRow = (Vec<u8>, String, Option<u16>, Option<Vec<u8>>);

/// In-memory idempotency repo backed by a HashMap — captures state for assertions.
struct MemIdempotencyRepo {
    rows: tokio::sync::Mutex<std::collections::HashMap<(Uuid, String), MemIdempotencyRow>>,
}

impl MemIdempotencyRepo {
    fn new() -> Self {
        Self {
            rows: tokio::sync::Mutex::new(Default::default()),
        }
    }
}

#[async_trait]
impl IdempotencyRepository for MemIdempotencyRepo {
    async fn begin(
        &self,
        api_key_id: Uuid,
        key: &str,
        request_hash: &[u8; 32],
        _method: &str,
        _path: &str,
    ) -> Result<IdempotencyOutcome, DbError> {
        let mut rows = self.rows.lock().await;
        let k = (api_key_id, key.to_string());
        match rows.get(&k) {
            None => {
                rows.insert(k, (request_hash.to_vec(), "in_progress".into(), None, None));
                Ok(IdempotencyOutcome::Fresh)
            }
            Some((existing_hash, status, response_status, response_body)) => {
                if existing_hash != &request_hash[..] {
                    Ok(IdempotencyOutcome::HashMismatch)
                } else if status == "completed" {
                    Ok(IdempotencyOutcome::Replay {
                        status: response_status.unwrap_or(0),
                        body: response_body.clone().unwrap_or_default(),
                    })
                } else {
                    Err(DbError::Internal(anyhow::anyhow!(
                        "in_progress without commit — test sequencing bug"
                    )))
                }
            }
        }
    }

    async fn complete(
        &self,
        api_key_id: Uuid,
        key: &str,
        response_status: u16,
        response_body: &[u8],
    ) -> Result<(), DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(row) = rows.get_mut(&(api_key_id, key.to_string())) {
            row.1 = "completed".into();
            row.2 = Some(response_status);
            row.3 = Some(response_body.to_vec());
        }
        Ok(())
    }

    async fn delete_expired(&self, _limit: i64) -> Result<u64, DbError> {
        Ok(0)
    }
}

/// Build an AppState wired with a real MemIdempotencyRepo.
fn fixture_with_mem_idempotency() -> (Arc<AppState>, Arc<MockMessageRepo>) {
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

    let messages = Arc::new(MockMessageRepo::new());
    let suppressions = Arc::new(MockSuppressionRepo::new());
    let api_key_repo = Arc::new(MockApiKeyRepo);
    let provider_config_repo = Arc::new(MockProviderConfigRepo);
    let webhook_repo = Arc::new(MockWebhookRepo);
    let idempotency_repo: Arc<dyn IdempotencyRepository> = Arc::new(MemIdempotencyRepo::new());
    let verification_repo: Arc<dyn VerificationRepository> = Arc::new(NullVerificationRepo);
    let totp_repo: Arc<dyn TotpRepository> = Arc::new(NullTotpRepo);
    let encryptor = null_encryptor();

    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let config = Arc::new(Config::from_env());

    let state = Arc::new(AppState::with_repos(
        redis,
        config,
        account_repo,
        messages.clone(),
        api_key_repo,
        provider_config_repo,
        webhook_repo,
        suppressions,
        idempotency_repo,
        verification_repo,
        totp_repo,
        encryptor,
    ));

    (state, messages)
}

/// Seed a suppression for the given recipient via the public HTTP API,
/// so subsequent `/v1/sms/send` calls hit the 422 short-circuit path
/// (no message_repo insert, no Redis enqueue — keeps tests offline).
async fn seed_sms_suppression(app: &axum::Router, recipient: &str) {
    let body = serde_json::json!({"channel":"sms","recipient":recipient}).to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success() || resp.status() == StatusCode::CONFLICT);
}

#[tokio::test]
async fn sms_send_with_idempotency_key_caches_and_replays() {
    // Use the suppressed-recipient path so we exercise idempotency without
    // requiring Redis for the enqueue step.
    let (state, msg_repo) = fixture_with_mem_idempotency();
    let app = create_router(state);
    seed_sms_suppression(&app, "+66812345678").await;

    let body = serde_json::json!({"to":"+66812345678","body":"hi"}).to_string();
    let key = "test-key-1";

    let resp1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes1 = resp1.into_body().collect().await.unwrap().to_bytes();

    let resp2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();

    assert_eq!(bytes1, bytes2, "replay must be byte-for-byte identical");
    assert_eq!(
        msg_repo.messages.lock().unwrap().len(),
        0,
        "suppressed sends must not create messages on either call"
    );
}

#[tokio::test]
async fn sms_send_with_same_key_different_body_returns_422_idempotency_key_reused() {
    // First request returns 422 (suppression). Second request with same key but
    // different body must return 422 with code=idempotency_key_reused (not
    // recipient_suppressed) — proving hash mismatch is detected.
    let (state, _msg_repo) = fixture_with_mem_idempotency();
    let app = create_router(state);
    seed_sms_suppression(&app, "+66812345678").await;
    let key = "test-key-2";

    let resp1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({"to":"+66812345678","body":"A"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let resp2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({"to":"+66812345678","body":"B"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let v = response_body(resp2).await;
    assert_eq!(v["error"]["code"], "idempotency_key_reused");
}

#[tokio::test]
async fn sms_send_with_invalid_idempotency_header_returns_400() {
    let (state, _msg_repo) = fixture_with_mem_idempotency();
    let app = create_router(state);
    let body = serde_json::json!({"to":"+66812345678","body":"hi"}).to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", "")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_body(resp).await;
    assert_eq!(v["error"]["code"], "invalid_idempotency_key");

    let long = "a".repeat(256);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", long)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sms_send_without_idempotency_key_keeps_existing_behavior() {
    // Without the header, behavior should match the legacy path — including
    // returning 422 for a suppressed recipient.
    let (state, _msg_repo) = fixture_with_mem_idempotency();
    let app = create_router(state);
    seed_sms_suppression(&app, "+66812345678").await;
    let body = serde_json::json!({"to":"+66812345678","body":"hi"}).to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// Seed an email suppression so /v1/email/send hits the 422 short-circuit.
async fn seed_email_suppression(app: &axum::Router, recipient: &str) {
    let body = serde_json::json!({"channel":"email","recipient":recipient}).to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success() || resp.status() == StatusCode::CONFLICT);
}

#[tokio::test]
async fn email_send_with_idempotency_key_caches_and_replays() {
    let (state, msg_repo) = fixture_with_mem_idempotency();
    let app = create_router(state);
    seed_email_suppression(&app, "alice@example.com").await;

    let body = serde_json::json!({
        "to":"alice@example.com",
        "subject":"hello",
        "body":"hi"
    })
    .to_string();
    let key = "email-key-1";

    let resp1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes1 = resp1.into_body().collect().await.unwrap().to_bytes();

    let resp2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/email/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();

    assert_eq!(bytes1, bytes2);
    assert_eq!(msg_repo.messages.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn otp_send_with_idempotency_key_replays() {
    // Use suppressed recipient so we don't need Redis for otp::store.
    let (state, _msg_repo) = fixture_with_mem_idempotency();
    let app = create_router(state);
    seed_sms_suppression(&app, "+66812345678").await;

    let body = serde_json::json!({"to":"+66812345678"}).to_string();
    let key = "otp-key-1";

    let resp1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/otp/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes1 = resp1.into_body().collect().await.unwrap().to_bytes();

    let resp2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/otp/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();

    assert_eq!(bytes1, bytes2, "OTP replay must be byte-for-byte identical");
}

#[tokio::test]
async fn sms_batch_with_idempotency_key_replays_full_partition() {
    let (state, _msg_repo) = fixture_with_mem_idempotency();
    let app = create_router(state);
    seed_sms_suppression(&app, "+66811111111").await;
    seed_sms_suppression(&app, "+66822222222").await;

    let body = serde_json::json!({
        "from": null,
        "recipients": [
            {"to":"+66811111111","body":"a"},
            {"to":"+66822222222","body":"b"}
        ]
    })
    .to_string();
    let key = "batch-key-1";

    let resp1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send-batch")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::MULTI_STATUS);
    let bytes1 = resp1.into_body().collect().await.unwrap().to_bytes();

    let resp2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send-batch")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::MULTI_STATUS);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();

    assert_eq!(
        bytes1, bytes2,
        "batch replay must be byte-for-byte identical"
    );
}

// ----- C1 cross-API-key isolation -----

const TEST_API_KEY_A: &str =
    "ch_test_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TEST_API_KEY_B: &str =
    "ch_test_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct MockMultiKeyAccountRepo {
    account: Account,
    keys: std::collections::HashMap<String, ApiKey>,
}

#[async_trait]
impl AccountRepository for MockMultiKeyAccountRepo {
    async fn find_by_api_key_hash(&self, hash: &str) -> Result<Option<(Account, ApiKey)>, DbError> {
        Ok(self
            .keys
            .get(hash)
            .map(|k| (self.account.clone(), k.clone())))
    }

    async fn update_key_last_used(&self, _key_id: Uuid) -> Result<(), DbError> {
        Ok(())
    }
}

fn fixture_two_api_keys() -> Arc<AppState> {
    let account_id = Uuid::new_v4();
    let account = Account {
        id: account_id,
        name: "Test Account".into(),
        owner_email: "test@example.com".into(),
        is_active: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let mut keys = std::collections::HashMap::new();
    keys.insert(
        hex::encode(Sha256::digest(TEST_API_KEY_A.as_bytes())),
        ApiKey {
            id: Uuid::new_v4(),
            account_id,
            name: "key A".into(),
            key_prefix: "ch_test_aa...".into(),
            environment: "test".into(),
            last_used_at: None,
            expires_at: None,
            is_revoked: false,
            created_at: Utc::now(),
        },
    );
    keys.insert(
        hex::encode(Sha256::digest(TEST_API_KEY_B.as_bytes())),
        ApiKey {
            id: Uuid::new_v4(),
            account_id,
            name: "key B".into(),
            key_prefix: "ch_test_bb...".into(),
            environment: "test".into(),
            last_used_at: None,
            expires_at: None,
            is_revoked: false,
            created_at: Utc::now(),
        },
    );

    let account_repo = Arc::new(MockMultiKeyAccountRepo { account, keys });
    let messages = Arc::new(MockMessageRepo::new());
    let suppressions = Arc::new(MockSuppressionRepo::new());
    let api_key_repo = Arc::new(MockApiKeyRepo);
    let provider_config_repo = Arc::new(MockProviderConfigRepo);
    let webhook_repo = Arc::new(MockWebhookRepo);
    let idempotency_repo: Arc<dyn IdempotencyRepository> = Arc::new(MemIdempotencyRepo::new());
    let verification_repo: Arc<dyn VerificationRepository> = Arc::new(NullVerificationRepo);
    let totp_repo: Arc<dyn TotpRepository> = Arc::new(NullTotpRepo);
    let encryptor = null_encryptor();

    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let config = Arc::new(Config::from_env());

    Arc::new(AppState::with_repos(
        redis,
        config,
        account_repo,
        messages,
        api_key_repo,
        provider_config_repo,
        webhook_repo,
        suppressions,
        idempotency_repo,
        verification_repo,
        totp_repo,
        encryptor,
    ))
}

#[tokio::test]
async fn idempotency_is_isolated_by_api_key() {
    // The same Idempotency-Key value sent under two different API keys must
    // NOT collide — so reusing the value with a different body under api_key
    // B must succeed (422 suppression), not return 422 idempotency_key_reused.
    let state = fixture_two_api_keys();
    let app = create_router(state);

    // Seed suppression via key A — it doesn't matter which key seeds, the
    // suppression list is per-account.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY_A}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"sms","recipient":"+66812345678"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success() || resp.status() == StatusCode::CONFLICT);

    let key = "shared-idem-key";

    // Call 1 — API key A, body=A
    let resp_a = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY_A}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({"to":"+66812345678","body":"A"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_a.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let v_a = response_body(resp_a).await;
    assert_eq!(v_a["error"]["code"], "recipient_suppressed");

    // Call 2 — API key B, body=B (would collide with A under shared scope)
    let resp_b = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/sms/send")
                .header("authorization", format!("Bearer {TEST_API_KEY_B}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({"to":"+66812345678","body":"B"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_b.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let v_b = response_body(resp_b).await;
    // If isolation works, B sees its own fresh slot → suppression check
    // fires → recipient_suppressed. If broken, it would be idempotency_key_reused.
    assert_eq!(
        v_b["error"]["code"], "recipient_suppressed",
        "same Idempotency-Key under a different api_key must not collide"
    );
}

// ----- B1 verification tests -----

struct MemVerificationRepo {
    rows: tokio::sync::Mutex<Vec<Verification>>,
}

impl MemVerificationRepo {
    fn new() -> Self {
        Self {
            rows: tokio::sync::Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl VerificationRepository for MemVerificationRepo {
    async fn insert(&self, v: &NewVerification) -> Result<Verification, DbError> {
        let now = Utc::now();
        let row = Verification {
            id: Uuid::new_v4(),
            account_id: v.account_id,
            api_key_id: v.api_key_id,
            channel: v.channel.clone(),
            recipient: v.recipient.clone(),
            status: "pending".into(),
            check_attempts: 0,
            resend_attempts: 0,
            cost_micro: v.initial_cost_micro,
            cost_currency: "USD".into(),
            environment: v.environment.clone(),
            app_name: v.app_name.clone(),
            created_at: now,
            updated_at: now,
            expires_at: now + chrono::Duration::seconds(300),
        };
        self.rows.lock().await.push(row.clone());
        Ok(row)
    }
    async fn find_by_id(
        &self,
        id: Uuid,
        account_id: Uuid,
    ) -> Result<Option<Verification>, DbError> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .find(|v| v.id == id && v.account_id == account_id)
            .cloned())
    }
    async fn list_by_account(
        &self,
        account_id: Uuid,
        pagination: &chorus_server::db::Pagination,
    ) -> Result<Vec<Verification>, DbError> {
        let rows = self.rows.lock().await;
        Ok(rows
            .iter()
            .filter(|v| v.account_id == account_id)
            .skip(pagination.offset as usize)
            .take(pagination.limit as usize)
            .cloned()
            .collect())
    }
    async fn increment_check_attempts(&self, id: Uuid, account_id: Uuid) -> Result<i32, DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(v) = rows
            .iter_mut()
            .find(|v| v.id == id && v.account_id == account_id && v.status == "pending")
        {
            v.check_attempts += 1;
            return Ok(v.check_attempts);
        }
        Err(DbError::NotFound)
    }
    async fn mark_approved(&self, id: Uuid, account_id: Uuid) -> Result<(), DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(v) = rows
            .iter_mut()
            .find(|v| v.id == id && v.account_id == account_id && v.status == "pending")
        {
            v.status = "approved".into();
            return Ok(());
        }
        Err(DbError::NotFound)
    }
    async fn mark_canceled(&self, id: Uuid, account_id: Uuid) -> Result<bool, DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(v) = rows
            .iter_mut()
            .find(|v| v.id == id && v.account_id == account_id && v.status == "pending")
        {
            v.status = "canceled".into();
            return Ok(true);
        }
        Ok(false)
    }
    async fn record_resend(
        &self,
        id: Uuid,
        account_id: Uuid,
        additional_cost_micro: i64,
        max_resends: i32,
    ) -> Result<Verification, DbError> {
        let mut rows = self.rows.lock().await;
        if let Some(v) = rows.iter_mut().find(|v| {
            v.id == id
                && v.account_id == account_id
                && v.status == "pending"
                && v.resend_attempts < max_resends
        }) {
            v.resend_attempts += 1;
            v.cost_micro += additional_cost_micro;
            v.check_attempts = 0;
            return Ok(v.clone());
        }
        Err(DbError::NotFound)
    }
    async fn expire_pending(&self, _limit: i64) -> Result<u64, DbError> {
        Ok(0)
    }
}

/// Build an AppState wired with both MemIdempotencyRepo and MemVerificationRepo.
fn fixture_with_verification() -> (Arc<AppState>, Arc<MemVerificationRepo>) {
    let key_hash = hex::encode(Sha256::digest(TEST_API_KEY.as_bytes()));
    let account_id = Uuid::new_v4();
    let key_id = Uuid::new_v4();
    let account_repo = Arc::new(MockAccountRepo {
        account: Account {
            id: account_id,
            name: "Test".into(),
            owner_email: "t@t".into(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        api_key: ApiKey {
            id: key_id,
            account_id,
            name: "k".into(),
            key_prefix: "ch_test_ab...".into(),
            environment: "test".into(),
            last_used_at: None,
            expires_at: None,
            is_revoked: false,
            created_at: Utc::now(),
        },
        key_hash,
    });
    let messages = Arc::new(MockMessageRepo::new());
    let suppressions = Arc::new(MockSuppressionRepo::new());
    let api_key_repo = Arc::new(MockApiKeyRepo);
    let provider_config_repo = Arc::new(MockProviderConfigRepo);
    let webhook_repo = Arc::new(MockWebhookRepo);
    let idempotency_repo: Arc<dyn IdempotencyRepository> = Arc::new(MemIdempotencyRepo::new());
    let verification_repo: Arc<MemVerificationRepo> = Arc::new(MemVerificationRepo::new());
    let verification_dyn: Arc<dyn VerificationRepository> = verification_repo.clone();
    let totp_repo: Arc<dyn TotpRepository> = Arc::new(NullTotpRepo);
    let encryptor = null_encryptor();
    let redis = redis::Client::open("redis://127.0.0.1:6379").unwrap();
    let config = Arc::new(Config::from_env());
    let state = Arc::new(AppState::with_repos(
        redis,
        config,
        account_repo,
        messages,
        api_key_repo,
        provider_config_repo,
        webhook_repo,
        suppressions,
        idempotency_repo,
        verification_dyn,
        totp_repo,
        encryptor,
    ));
    (state, verification_repo)
}

#[tokio::test]
#[ignore = "requires Valkey/Redis on localhost:6379"]
async fn create_verification_with_email_returns_201_with_email_channel() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let body = serde_json::json!({
        "phone": "+66812345678",
        "email": "alice@example.com",
        "app_name": "Acme"
    })
    .to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = response_body(resp).await;
    assert_eq!(v["channel"], "email");
    assert_eq!(v["recipient"], "alice@example.com");
    assert_eq!(v["cost_micro"], 100);
    assert_eq!(v["status"], "pending");
}

#[tokio::test]
async fn create_verification_returns_400_when_no_recipient() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_body(resp).await;
    assert_eq!(v["error"]["code"], "no_recipient");
}

#[tokio::test]
async fn create_verification_returns_400_when_invalid_phone() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let body = serde_json::json!({ "phone": "0812345678" }).to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response_body(resp).await["error"]["code"], "invalid_phone");
}

#[tokio::test]
async fn create_verification_returns_422_when_no_eligible_channel() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state.clone());
    // Suppress both
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"alice@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"sms","recipient":"+66812345678"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let body = serde_json::json!({
        "phone": "+66812345678",
        "email": "alice@example.com",
    })
    .to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response_body(resp).await["error"]["code"],
        "no_eligible_channel"
    );
}

#[tokio::test]
#[ignore = "requires Valkey/Redis on localhost:6379"]
async fn create_verification_falls_back_to_sms_when_email_suppressed() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"email","recipient":"alice@example.com"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = serde_json::json!({
        "phone": "+66812345678",
        "email": "alice@example.com",
    })
    .to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = response_body(resp).await;
    assert_eq!(v["channel"], "sms");
    assert_eq!(v["recipient"], "+66812345678");
    assert_eq!(v["cost_micro"], 6000); // TH
}

// helper: get the most recently created verification id from the in-memory repo
async fn last_verification_id(repo: &Arc<MemVerificationRepo>) -> Uuid {
    repo.rows.lock().await.last().expect("no verifications").id
}

// helper to seed a code into Valkey for /check tests (only useful when Valkey is running)
async fn seed_pending_code(state: &Arc<AppState>, id: Uuid, code: &str) {
    let _ = chorus_server::verification::store_code(&state.redis, id, "test", code).await;
}

#[tokio::test]
#[ignore = "requires Valkey/Redis on localhost:6379"]
async fn check_with_correct_code_returns_approved() {
    let (state, repo) = fixture_with_verification();
    let app = create_router(state.clone());

    let body = serde_json::json!({"email":"a@b.com"}).to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let id = last_verification_id(&repo).await;

    seed_pending_code(&state, id, "999111").await;

    let check_body = serde_json::json!({"code":"999111"}).to_string();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/verifications/{id}/check"))
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(check_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(response_body(resp).await["status"], "approved");
}

#[tokio::test]
#[ignore = "requires Valkey/Redis on localhost:6379"]
async fn check_with_wrong_code_returns_422_with_attempts_remaining() {
    let (state, repo) = fixture_with_verification();
    let app = create_router(state.clone());
    let body = serde_json::json!({"email":"a@b.com"}).to_string();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let id = last_verification_id(&repo).await;
    seed_pending_code(&state, id, "111222").await;

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/verifications/{id}/check"))
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(r#"{"code":"000000"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let v = response_body(resp).await;
    assert_eq!(v["error"]["code"], "incorrect_code");
    assert_eq!(v["error"]["attempts_remaining"], 4);
}

#[tokio::test]
#[ignore = "requires Valkey/Redis on localhost:6379"]
async fn cancel_pending_returns_canceled() {
    let (state, repo) = fixture_with_verification();
    let app = create_router(state);
    let body = serde_json::json!({"email":"a@b.com"}).to_string();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let id = last_verification_id(&repo).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/verifications/{id}/cancel"))
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(response_body(resp).await["status"], "canceled");

    // Second cancel → 410
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/verifications/{id}/cancel"))
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

#[tokio::test]
#[ignore = "requires Valkey/Redis on localhost:6379"]
async fn get_returns_verification_and_404_for_unknown() {
    let (state, repo) = fixture_with_verification();
    let app = create_router(state);
    let body = serde_json::json!({"email":"a@b.com"}).to_string();
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let id = last_verification_id(&repo).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/verifications/{id}"))
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let other = Uuid::new_v4();
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/verifications/{other}"))
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Valkey/Redis on localhost:6379"]
async fn list_paginates() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    for i in 0..3 {
        let body = serde_json::json!({"email": format!("u{i}@x.com")}).to_string();
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/verifications")
                    .header("authorization", format!("Bearer {TEST_API_KEY}"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
    }
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/verifications?limit=2")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = response_body(resp).await;
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
    assert_eq!(v["limit"], 2);
}

#[tokio::test]
#[ignore = "requires Valkey/Redis on localhost:6379"]
async fn create_verification_idempotency_replay_is_identical() {
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    let key = "verify-key-1";
    let body = serde_json::json!({"email":"alice@example.com"}).to_string();

    let resp1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::CREATED);
    let bytes1 = resp1.into_body().collect().await.unwrap().to_bytes();

    let resp2 = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/verifications")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("Idempotency-Key", key)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::CREATED);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(bytes1, bytes2);
}

#[tokio::test]
async fn legacy_otp_send_still_works() {
    // Regression — C1 idempotency wiring on /v1/otp/send must be intact.
    let (state, _repo) = fixture_with_verification();
    let app = create_router(state);
    // Pre-suppress recipient to skip Redis enqueue (mirrors C1 test pattern).
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/suppressions")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"channel":"sms","recipient":"+66812345678"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/otp/send")
                .header("authorization", format!("Bearer {TEST_API_KEY}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(r#"{"to":"+66812345678"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    // Legacy route returns 422 due to suppression — works as before.
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
