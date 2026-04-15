use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::DbError;

/// A billing plan (free, starter, pro, enterprise).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BillingPlan {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub price_cents: i32,
    pub billing_period: String,
    pub sms_quota: i32,
    pub email_quota: i32,
    pub created_at: DateTime<Utc>,
}

/// An account's subscription to a billing plan.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Subscription {
    pub id: Uuid,
    pub account_id: Uuid,
    pub plan_id: Uuid,
    pub stripe_customer_id: Option<String>,
    pub stripe_subscription_id: Option<String>,
    pub status: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Usage counters for a billing period.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Usage {
    pub id: Uuid,
    pub account_id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub sms_sent: i32,
    pub email_sent: i32,
    pub sms_cost_microdollars: i64,
    pub email_cost_microdollars: i64,
    pub created_at: DateTime<Utc>,
}

/// Billing data operations.
#[async_trait]
pub trait BillingRepository: Send + Sync {
    /// List all available billing plans.
    async fn list_plans(&self) -> Result<Vec<BillingPlan>, DbError>;

    /// Get a plan by slug (e.g., "free", "starter", "pro").
    async fn get_plan_by_slug(&self, slug: &str) -> Result<Option<BillingPlan>, DbError>;

    /// Get the active subscription for an account.
    async fn get_subscription(&self, account_id: Uuid) -> Result<Option<Subscription>, DbError>;

    /// Create or update a subscription.
    async fn upsert_subscription(
        &self,
        account_id: Uuid,
        plan_id: Uuid,
        stripe_customer_id: Option<&str>,
        stripe_subscription_id: Option<&str>,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<Subscription, DbError>;

    /// Update subscription status (active, past_due, canceled).
    async fn update_subscription_status(
        &self,
        account_id: Uuid,
        status: &str,
    ) -> Result<(), DbError>;

    /// Get current period usage for an account.
    async fn get_usage(&self, account_id: Uuid) -> Result<Option<Usage>, DbError>;
}

/// PostgreSQL implementation of `BillingRepository`.
pub struct PgBillingRepository {
    db: PgPool,
}

impl PgBillingRepository {
    /// Create from a database pool.
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

#[async_trait]
impl BillingRepository for PgBillingRepository {
    async fn list_plans(&self) -> Result<Vec<BillingPlan>, DbError> {
        sqlx::query_as::<_, BillingPlan>("SELECT * FROM billing_plans ORDER BY price_cents")
            .fetch_all(&self.db)
            .await
            .map_err(|e| DbError::Internal(e.into()))
    }

    async fn get_plan_by_slug(&self, slug: &str) -> Result<Option<BillingPlan>, DbError> {
        sqlx::query_as::<_, BillingPlan>("SELECT * FROM billing_plans WHERE slug = $1")
            .bind(slug)
            .fetch_optional(&self.db)
            .await
            .map_err(|e| DbError::Internal(e.into()))
    }

    async fn get_subscription(&self, account_id: Uuid) -> Result<Option<Subscription>, DbError> {
        sqlx::query_as::<_, Subscription>(
            "SELECT * FROM subscriptions WHERE account_id = $1 AND status = 'active' LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| DbError::Internal(e.into()))
    }

    async fn upsert_subscription(
        &self,
        account_id: Uuid,
        plan_id: Uuid,
        stripe_customer_id: Option<&str>,
        stripe_subscription_id: Option<&str>,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
    ) -> Result<Subscription, DbError> {
        let sub = sqlx::query_as::<_, Subscription>(
            r#"
            INSERT INTO subscriptions (account_id, plan_id, stripe_customer_id, stripe_subscription_id, period_start, period_end)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (account_id) DO UPDATE SET
                plan_id = EXCLUDED.plan_id,
                stripe_customer_id = EXCLUDED.stripe_customer_id,
                stripe_subscription_id = EXCLUDED.stripe_subscription_id,
                period_start = EXCLUDED.period_start,
                period_end = EXCLUDED.period_end,
                status = 'active'
            RETURNING *
            "#,
        )
        .bind(account_id)
        .bind(plan_id)
        .bind(stripe_customer_id)
        .bind(stripe_subscription_id)
        .bind(period_start)
        .bind(period_end)
        .fetch_one(&self.db)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        // Ensure usage row exists for the period
        sqlx::query(
            r#"
            INSERT INTO usage (account_id, period_start, period_end)
            VALUES ($1, $2, $3)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(account_id)
        .bind(period_start)
        .bind(period_end)
        .execute(&self.db)
        .await
        .map_err(|e| DbError::Internal(e.into()))?;

        Ok(sub)
    }

    async fn update_subscription_status(
        &self,
        account_id: Uuid,
        status: &str,
    ) -> Result<(), DbError> {
        sqlx::query("UPDATE subscriptions SET status = $1 WHERE account_id = $2 AND status = 'active'")
            .bind(status)
            .bind(account_id)
            .execute(&self.db)
            .await
            .map_err(|e| DbError::Internal(e.into()))?;
        Ok(())
    }

    async fn get_usage(&self, account_id: Uuid) -> Result<Option<Usage>, DbError> {
        sqlx::query_as::<_, Usage>(
            "SELECT * FROM usage WHERE account_id = $1 AND period_start <= now() AND period_end > now() LIMIT 1",
        )
        .bind(account_id)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| DbError::Internal(e.into()))
    }
}

/// No-op billing repository for tests and self-hosted mode.
pub struct NullBillingRepository;

#[async_trait]
impl BillingRepository for NullBillingRepository {
    async fn list_plans(&self) -> Result<Vec<BillingPlan>, DbError> {
        Ok(vec![])
    }
    async fn get_plan_by_slug(&self, _slug: &str) -> Result<Option<BillingPlan>, DbError> {
        Ok(None)
    }
    async fn get_subscription(&self, _account_id: Uuid) -> Result<Option<Subscription>, DbError> {
        Ok(None)
    }
    async fn upsert_subscription(
        &self,
        _account_id: Uuid,
        _plan_id: Uuid,
        _stripe_customer_id: Option<&str>,
        _stripe_subscription_id: Option<&str>,
        _period_start: DateTime<Utc>,
        _period_end: DateTime<Utc>,
    ) -> Result<Subscription, DbError> {
        Err(DbError::NotFound)
    }
    async fn update_subscription_status(
        &self,
        _account_id: Uuid,
        _status: &str,
    ) -> Result<(), DbError> {
        Ok(())
    }
    async fn get_usage(&self, _account_id: Uuid) -> Result<Option<Usage>, DbError> {
        Ok(None)
    }
}
