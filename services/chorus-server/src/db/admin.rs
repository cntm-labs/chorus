use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{DbError, Message, Webhook};
use crate::routes::admin::accounts::{AccountDetail, AccountListItem};
use crate::routes::admin::billing::{BillingAccountSummary, BillingReport, PlanCount};
use crate::routes::admin::messages::MessageSearchFilters;
use crate::routes::admin::providers::{AdminProviderConfig, ProviderHealth};
use crate::routes::admin::webhooks::{AdminWebhook, WebhookDelivery};

/// Repository for admin-only cross-account queries.
#[async_trait]
pub trait AdminRepository: Send + Sync {
    /// List all accounts.
    async fn list_accounts(&self) -> Result<Vec<AccountListItem>, DbError>;
    /// Get account detail with usage stats.
    async fn get_account_detail(&self, id: Uuid) -> Result<Option<AccountDetail>, DbError>;
    /// Create a new account.
    async fn create_account(&self, name: &str, email: &str) -> Result<AccountListItem, DbError>;
    /// Update account fields.
    async fn update_account(
        &self,
        id: Uuid,
        is_active: Option<bool>,
        name: Option<&str>,
    ) -> Result<(), DbError>;
    /// Deactivate (soft-delete) an account.
    async fn deactivate_account(&self, id: Uuid) -> Result<(), DbError>;

    // --- Provider Config (#35) ---

    /// List all provider configs across accounts.
    async fn list_all_provider_configs(&self) -> Result<Vec<AdminProviderConfig>, DbError>;
    /// Get provider health summary (error rate from recent delivery events).
    async fn get_provider_health(&self, id: Uuid) -> Result<Option<ProviderHealth>, DbError>;
    /// Update provider config (priority, is_active).
    async fn update_provider_config(
        &self,
        id: Uuid,
        priority: Option<i32>,
        is_active: Option<bool>,
    ) -> Result<(), DbError>;
    /// Disable a provider across all accounts (outage scenario).
    async fn disable_provider_by_name(&self, provider: &str) -> Result<u64, DbError>;

    // --- DLQ + Message Inspector (#36, #37) ---

    /// Get a message by ID without account scoping (admin-only).
    async fn get_message_by_id(&self, id: Uuid) -> Result<Option<Message>, DbError>;
    /// Search messages across all accounts with filters.
    async fn search_messages(
        &self,
        filters: &MessageSearchFilters,
    ) -> Result<Vec<Message>, DbError>;

    // --- Billing (#38) ---

    /// List all accounts with billing status.
    async fn list_billing_accounts(&self) -> Result<Vec<BillingAccountSummary>, DbError>;
    /// Override an account's subscription plan.
    async fn override_plan(&self, account_id: Uuid, plan_slug: &str) -> Result<(), DbError>;
    /// Adjust usage counters.
    async fn adjust_usage(
        &self,
        account_id: Uuid,
        sms_delta: Option<i32>,
        email_delta: Option<i32>,
    ) -> Result<(), DbError>;
    /// Generate billing report.
    async fn billing_report(&self) -> Result<BillingReport, DbError>;

    // --- Webhook (#39) ---

    /// List all webhooks across all accounts.
    async fn list_all_webhooks(&self) -> Result<Vec<AdminWebhook>, DbError>;
    /// Get a webhook by ID (admin, no account scoping).
    async fn get_webhook_by_id(&self, id: Uuid) -> Result<Option<Webhook>, DbError>;
    /// Get webhook delivery log.
    async fn get_webhook_deliveries(
        &self,
        webhook_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WebhookDelivery>, DbError>;
    /// Enable/disable a webhook.
    async fn update_webhook_status(&self, id: Uuid, is_active: bool) -> Result<(), DbError>;
    /// Disable all webhooks for an account.
    async fn disable_account_webhooks(&self, account_id: Uuid) -> Result<u64, DbError>;
}

/// PostgreSQL implementation of admin repository.
pub struct PgAdminRepository {
    pool: PgPool,
}

impl PgAdminRepository {
    /// Create a new admin repository.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AdminRepository for PgAdminRepository {
    async fn list_accounts(&self) -> Result<Vec<AccountListItem>, DbError> {
        let rows = sqlx::query_as::<_, AccountListItem>(
            "SELECT id, name, owner_email, is_active, created_at
             FROM accounts ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(rows)
    }

    async fn get_account_detail(&self, id: Uuid) -> Result<Option<AccountDetail>, DbError> {
        let row = sqlx::query_as::<_, AccountDetail>(
            "SELECT a.id, a.name, a.owner_email, a.is_active, a.created_at, a.updated_at,
                    (SELECT COUNT(*) FROM api_keys WHERE account_id = a.id) AS key_count,
                    (SELECT COUNT(*) FROM messages WHERE account_id = a.id) AS message_count
             FROM accounts a WHERE a.id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(row)
    }

    async fn create_account(&self, name: &str, email: &str) -> Result<AccountListItem, DbError> {
        let row = sqlx::query_as::<_, AccountListItem>(
            "INSERT INTO accounts (name, owner_email) VALUES ($1, $2)
             RETURNING id, name, owner_email, is_active, created_at",
        )
        .bind(name)
        .bind(email)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(row)
    }

    async fn update_account(
        &self,
        id: Uuid,
        is_active: Option<bool>,
        name: Option<&str>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE accounts SET
                is_active = COALESCE($1, is_active),
                name = COALESCE($2, name),
                updated_at = now()
             WHERE id = $3",
        )
        .bind(is_active)
        .bind(name)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn deactivate_account(&self, id: Uuid) -> Result<(), DbError> {
        sqlx::query("UPDATE accounts SET is_active = false, updated_at = now() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn list_all_provider_configs(&self) -> Result<Vec<AdminProviderConfig>, DbError> {
        let rows = sqlx::query_as::<_, AdminProviderConfig>(
            "SELECT id, account_id, channel, provider, priority, is_active, created_at
             FROM provider_configs ORDER BY account_id, priority",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(rows)
    }

    async fn get_provider_health(&self, id: Uuid) -> Result<Option<ProviderHealth>, DbError> {
        let row = sqlx::query_as::<_, ProviderHealth>(
            "SELECT pc.id, pc.provider,
                    COUNT(CASE WHEN m.status = 'delivered' THEN 1 END) AS total_sent,
                    COUNT(CASE WHEN m.status = 'failed' THEN 1 END) AS total_errors,
                    CASE
                        WHEN COUNT(*) FILTER (WHERE m.status IN ('delivered', 'failed')) = 0 THEN 0.0
                        ELSE COUNT(CASE WHEN m.status = 'failed' THEN 1 END)::float
                             / COUNT(*) FILTER (WHERE m.status IN ('delivered', 'failed'))
                    END AS error_rate,
                    MAX(CASE WHEN m.status = 'delivered' THEN m.delivered_at END) AS last_success,
                    MAX(CASE WHEN de.status = 'failed_attempt' THEN de.created_at END) AS last_error
             FROM provider_configs pc
             LEFT JOIN messages m ON m.provider = pc.provider AND m.account_id = pc.account_id
             LEFT JOIN delivery_events de ON de.message_id = m.id
             WHERE pc.id = $1
             GROUP BY pc.id, pc.provider",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(row)
    }

    async fn update_provider_config(
        &self,
        id: Uuid,
        priority: Option<i32>,
        is_active: Option<bool>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE provider_configs SET
                priority = COALESCE($1, priority),
                is_active = COALESCE($2, is_active)
             WHERE id = $3",
        )
        .bind(priority)
        .bind(is_active)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn disable_provider_by_name(&self, provider: &str) -> Result<u64, DbError> {
        let result =
            sqlx::query("UPDATE provider_configs SET is_active = false WHERE provider = $1")
                .bind(provider)
                .execute(&self.pool)
                .await
                .map_err(|e| DbError::Internal(e.into()))?;

        Ok(result.rows_affected())
    }

    async fn get_message_by_id(&self, id: Uuid) -> Result<Option<Message>, DbError> {
        let msg = sqlx::query_as::<_, Message>("SELECT * FROM messages WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| DbError::Internal(e.into()))?;

        Ok(msg)
    }

    async fn search_messages(
        &self,
        filters: &MessageSearchFilters,
    ) -> Result<Vec<Message>, DbError> {
        // Build dynamic query with conditional WHERE clauses
        let mut conditions = Vec::new();
        let mut param_idx = 1u32;

        if filters.account_id.is_some() {
            conditions.push(format!("account_id = ${param_idx}"));
            param_idx += 1;
        }
        if filters.channel.is_some() {
            conditions.push(format!("channel = ${param_idx}"));
            param_idx += 1;
        }
        if filters.status.is_some() {
            conditions.push(format!("status = ${param_idx}"));
            param_idx += 1;
        }
        if filters.provider.is_some() {
            conditions.push(format!("provider = ${param_idx}"));
            param_idx += 1;
        }
        if filters.date_from.is_some() {
            conditions.push(format!("created_at >= ${param_idx}"));
            param_idx += 1;
        }
        if filters.date_to.is_some() {
            conditions.push(format!("created_at <= ${param_idx}"));
            param_idx += 1;
        }
        if filters.recipient.is_some() {
            conditions.push(format!("recipient ILIKE ${param_idx}"));
            param_idx += 1;
        }
        if filters.min_cost.is_some() {
            conditions.push(format!("cost_microdollars >= ${param_idx}"));
            param_idx += 1;
        }
        if filters.max_cost.is_some() {
            conditions.push(format!("cost_microdollars <= ${param_idx}"));
            param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let limit_param = param_idx;
        param_idx += 1;
        let offset_param = param_idx;

        let sql = format!(
            "SELECT * FROM messages {where_clause} ORDER BY created_at DESC LIMIT ${limit_param} OFFSET ${offset_param}"
        );

        let mut query = sqlx::query_as::<_, Message>(&sql);

        // Bind parameters in the same order
        if let Some(ref v) = filters.account_id {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.channel {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.status {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.provider {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.date_from {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.date_to {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.recipient {
            query = query.bind(format!("%{v}%"));
        }
        if let Some(ref v) = filters.min_cost {
            query = query.bind(v);
        }
        if let Some(ref v) = filters.max_cost {
            query = query.bind(v);
        }

        let limit = filters.limit.unwrap_or(50);
        let offset = filters.offset.unwrap_or(0);
        query = query.bind(limit).bind(offset);

        let messages = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DbError::Internal(e.into()))?;

        Ok(messages)
    }

    async fn list_billing_accounts(&self) -> Result<Vec<BillingAccountSummary>, DbError> {
        let rows = sqlx::query_as::<_, BillingAccountSummary>(
            "SELECT a.id AS account_id, a.name AS account_name,
                    bp.slug AS plan_slug, s.status,
                    COALESCE(u.sms_sent, 0) AS sms_sent, bp.sms_quota,
                    COALESCE(u.email_sent, 0) AS email_sent, bp.email_quota,
                    s.period_end
             FROM accounts a
             JOIN subscriptions s ON s.account_id = a.id
             JOIN billing_plans bp ON bp.id = s.plan_id
             LEFT JOIN usage u ON u.account_id = a.id
                 AND u.period_start = s.period_start
             ORDER BY a.name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(rows)
    }

    async fn override_plan(&self, account_id: Uuid, plan_slug: &str) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE subscriptions SET plan_id = (SELECT id FROM billing_plans WHERE slug = $1)
             WHERE account_id = $2",
        )
        .bind(plan_slug)
        .bind(account_id)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn adjust_usage(
        &self,
        account_id: Uuid,
        sms_delta: Option<i32>,
        email_delta: Option<i32>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE usage SET
                sms_sent = sms_sent + COALESCE($1, 0),
                email_sent = email_sent + COALESCE($2, 0)
             WHERE account_id = $3
             AND period_start <= now() AND period_end > now()",
        )
        .bind(sms_delta)
        .bind(email_delta)
        .bind(account_id)
        .execute(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn billing_report(&self) -> Result<BillingReport, DbError> {
        // Total revenue from active subscriptions
        let revenue: (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(bp.price_cents), 0)
             FROM subscriptions s
             JOIN billing_plans bp ON bp.id = s.plan_id
             WHERE s.status = 'active'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        // Accounts by plan
        let accounts_by_plan = sqlx::query_as::<_, PlanCount>(
            "SELECT bp.slug AS plan_slug, COUNT(*) AS count
             FROM subscriptions s
             JOIN billing_plans bp ON bp.id = s.plan_id
             GROUP BY bp.slug ORDER BY count DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        // Accounts exceeding quotas
        let overage_rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT a.id
             FROM accounts a
             JOIN subscriptions s ON s.account_id = a.id
             JOIN billing_plans bp ON bp.id = s.plan_id
             LEFT JOIN usage u ON u.account_id = a.id
                 AND u.period_start = s.period_start
             WHERE (bp.sms_quota > 0 AND COALESCE(u.sms_sent, 0) > bp.sms_quota)
                OR (bp.email_quota > 0 AND COALESCE(u.email_sent, 0) > bp.email_quota)",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(BillingReport {
            total_revenue_cents: revenue.0,
            accounts_by_plan,
            overage_accounts: overage_rows.into_iter().map(|r| r.0).collect(),
        })
    }

    async fn list_all_webhooks(&self) -> Result<Vec<AdminWebhook>, DbError> {
        let rows = sqlx::query_as::<_, AdminWebhook>(
            "SELECT id, account_id, url, events, is_active, created_at
             FROM webhooks ORDER BY account_id, created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(rows)
    }

    async fn get_webhook_by_id(&self, id: Uuid) -> Result<Option<Webhook>, DbError> {
        let webhook = sqlx::query_as::<_, Webhook>(
            "SELECT id, account_id, url, secret, events, is_active, created_at
             FROM webhooks WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(webhook)
    }

    async fn get_webhook_deliveries(
        &self,
        webhook_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WebhookDelivery>, DbError> {
        let rows = sqlx::query_as::<_, WebhookDelivery>(
            "SELECT id, webhook_id, event, payload, response_status, response_body,
                    attempt, success, created_at
             FROM webhook_deliveries WHERE webhook_id = $1
             ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(webhook_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(rows)
    }

    async fn update_webhook_status(&self, id: Uuid, is_active: bool) -> Result<(), DbError> {
        sqlx::query("UPDATE webhooks SET is_active = $1 WHERE id = $2")
            .bind(is_active)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| DbError::Internal(e.into()))?;

        Ok(())
    }

    async fn disable_account_webhooks(&self, account_id: Uuid) -> Result<u64, DbError> {
        let result = sqlx::query("UPDATE webhooks SET is_active = false WHERE account_id = $1")
            .bind(account_id)
            .execute(&self.pool)
            .await
            .map_err(|e| DbError::Internal(e.into()))?;

        Ok(result.rows_affected())
    }
}
