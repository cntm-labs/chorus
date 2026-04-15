use stripe::{
    CheckoutSession, CheckoutSessionMode, Client, CreateCheckoutSession,
    CreateCheckoutSessionLineItems, CreateCheckoutSessionLineItemsPriceData,
    CreateCheckoutSessionLineItemsPriceDataProductData,
    CreateCheckoutSessionLineItemsPriceDataRecurring,
    CreateCheckoutSessionLineItemsPriceDataRecurringInterval, CreateCustomer, Currency, Customer,
};

/// Wrapper around the Stripe API client.
pub struct StripeClient {
    client: Client,
}

impl StripeClient {
    /// Create a new Stripe client from a secret key.
    pub fn new(secret_key: &str) -> Self {
        Self {
            client: Client::new(secret_key),
        }
    }

    /// Create a Stripe customer for an account.
    pub async fn create_customer(
        &self,
        email: &str,
        name: &str,
    ) -> Result<Customer, stripe::StripeError> {
        let mut params = CreateCustomer::new();
        params.email = Some(email);
        params.name = Some(name);
        Customer::create(&self.client, params).await
    }

    /// Create a Checkout Session for subscribing to a plan.
    pub async fn create_checkout_session(
        &self,
        customer_id: &str,
        plan_name: &str,
        price_cents: i64,
        success_url: &str,
        cancel_url: &str,
    ) -> Result<CheckoutSession, stripe::StripeError> {
        let mut params = CreateCheckoutSession::new();
        params.customer = Some(customer_id.parse().unwrap());
        params.mode = Some(CheckoutSessionMode::Subscription);
        params.success_url = Some(success_url);
        params.cancel_url = Some(cancel_url);
        params.line_items = Some(vec![CreateCheckoutSessionLineItems {
            quantity: Some(1),
            price_data: Some(CreateCheckoutSessionLineItemsPriceData {
                currency: Currency::USD,
                product_data: Some(CreateCheckoutSessionLineItemsPriceDataProductData {
                    name: plan_name.to_string(),
                    ..Default::default()
                }),
                unit_amount: Some(price_cents),
                recurring: Some(CreateCheckoutSessionLineItemsPriceDataRecurring {
                    interval: CreateCheckoutSessionLineItemsPriceDataRecurringInterval::Month,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }]);
        CheckoutSession::create(&self.client, params).await
    }
}
