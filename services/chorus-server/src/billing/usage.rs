use sqlx::PgPool;
use uuid::Uuid;

/// Increment usage counters for an account's current billing period.
pub async fn increment(db: &PgPool, account_id: Uuid, channel: &str) -> Result<(), sqlx::Error> {
    let (sms_inc, email_inc) = match channel {
        "sms" => (1_i32, 0_i32),
        "email" => (0, 1),
        _ => return Ok(()),
    };

    sqlx::query(
        r#"
        UPDATE usage
        SET sms_sent = sms_sent + $1,
            email_sent = email_sent + $2
        WHERE account_id = $3
          AND period_start <= now()
          AND period_end > now()
        "#,
    )
    .bind(sms_inc)
    .bind(email_inc)
    .bind(account_id)
    .execute(db)
    .await?;

    Ok(())
}

/// Check if an account has remaining quota for the given channel.
pub async fn check_quota(
    db: &PgPool,
    account_id: Uuid,
    channel: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_as::<_, (i32, i32, i32, i32)>(
        r#"
        SELECT u.sms_sent, u.email_sent, bp.sms_quota, bp.email_quota
        FROM usage u
        JOIN subscriptions s ON s.account_id = u.account_id
        JOIN billing_plans bp ON bp.id = s.plan_id
        WHERE u.account_id = $1
          AND u.period_start <= now()
          AND u.period_end > now()
          AND s.status = 'active'
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_optional(db)
    .await?;

    match row {
        Some((sms_sent, email_sent, sms_quota, email_quota)) => {
            // Enterprise plan (quota = 0) means unlimited
            let within_quota = match channel {
                "sms" => sms_quota == 0 || sms_sent < sms_quota,
                "email" => email_quota == 0 || email_sent < email_quota,
                _ => true,
            };
            Ok(within_quota)
        }
        // No subscription/usage record = allow (self-hosted free mode)
        None => Ok(true),
    }
}
