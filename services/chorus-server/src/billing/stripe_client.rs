use serde::{Deserialize, Serialize};

const STRIPE_API_BASE: &str = "https://api.stripe.com/v1";

/// Wrapper around the Stripe REST API using reqwest.
pub struct StripeClient {
    http: reqwest::Client,
    secret_key: String,
}

#[derive(Deserialize)]
pub struct StripeCustomer {
    pub id: String,
}

#[derive(Deserialize)]
pub struct StripeCheckoutSession {
    pub id: String,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StripeError {
    pub error: StripeErrorBody,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StripeErrorBody {
    pub message: String,
}

impl StripeClient {
    /// Create a new Stripe client from a secret key.
    pub fn new(secret_key: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            secret_key: secret_key.to_string(),
        }
    }

    /// Create a Stripe customer for an account.
    pub async fn create_customer(&self, email: &str, name: &str) -> Result<StripeCustomer, String> {
        let resp = self
            .http
            .post(format!("{STRIPE_API_BASE}/customers"))
            .basic_auth(&self.secret_key, None::<&str>)
            .form(&[("email", email), ("name", name)])
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let err: StripeError = resp.json().await.map_err(|e| e.to_string())?;
            return Err(err.error.message);
        }
        resp.json().await.map_err(|e| e.to_string())
    }

    /// Create a Checkout Session for subscribing to a plan.
    pub async fn create_checkout_session(
        &self,
        customer_id: &str,
        plan_name: &str,
        price_cents: i64,
        success_url: &str,
        cancel_url: &str,
    ) -> Result<StripeCheckoutSession, String> {
        let resp = self
            .http
            .post(format!("{STRIPE_API_BASE}/checkout/sessions"))
            .basic_auth(&self.secret_key, None::<&str>)
            .form(&[
                ("customer", customer_id),
                ("mode", "subscription"),
                ("success_url", success_url),
                ("cancel_url", cancel_url),
                ("line_items[0][quantity]", "1"),
                ("line_items[0][price_data][currency]", "usd"),
                ("line_items[0][price_data][product_data][name]", plan_name),
                (
                    "line_items[0][price_data][unit_amount]",
                    &price_cents.to_string(),
                ),
                ("line_items[0][price_data][recurring][interval]", "month"),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !resp.status().is_success() {
            let err: StripeError = resp.json().await.map_err(|e| e.to_string())?;
            return Err(err.error.message);
        }
        resp.json().await.map_err(|e| e.to_string())
    }
}
