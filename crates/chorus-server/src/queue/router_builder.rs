use chorus::router::WaterfallRouter;
use chorus_providers::email::mock::MockEmailSender;
use chorus_providers::email::resend::ResendEmailSender;
use chorus_providers::email::ses::SesEmailSender;
use chorus_providers::email::smtp::SmtpEmailSender;
use chorus_providers::sms::mock::MockSmsSender;
use chorus_providers::sms::plivo::PlivoSmsSender;
use chorus_providers::sms::telnyx::TelnyxSmsSender;
use chorus_providers::sms::twilio::TwilioSmsSender;
use std::sync::Arc;

use crate::config::Config;
use crate::db::ProviderConfig;

/// Build a router for test mode — always mock providers.
pub fn build_test_router(channel: &str) -> WaterfallRouter {
    let mut router = WaterfallRouter::new();
    match channel {
        "sms" => router = router.add_sms(Arc::new(MockSmsSender)),
        "email" => router = router.add_email(Arc::new(MockEmailSender)),
        _ => {}
    }
    router
}

/// Build a router from per-account provider configs (priority order).
pub fn build_router_from_configs(configs: &[ProviderConfig]) -> anyhow::Result<WaterfallRouter> {
    let mut router = WaterfallRouter::new();

    for config in configs {
        router = add_provider_to_router(
            router,
            &config.provider,
            &config.channel,
            &config.credentials,
        )?;
    }

    Ok(router)
}

/// Build a router from global env var defaults.
pub fn build_router_from_env(config: &Config, channel: &str) -> anyhow::Result<WaterfallRouter> {
    let mut router = WaterfallRouter::new();

    match channel {
        "sms" => {
            if let Some(ref api_key) = config.telnyx_api_key {
                router = router.add_sms(Arc::new(TelnyxSmsSender::new(
                    api_key.clone(),
                    config.telnyx_from.clone(),
                )));
            }
            if let (Some(ref sid), Some(ref token)) =
                (&config.twilio_account_sid, &config.twilio_auth_token)
            {
                router = router.add_sms(Arc::new(TwilioSmsSender::new(
                    sid.clone(),
                    token.clone(),
                    config.twilio_from.clone(),
                )));
            }
            if let (Some(ref id), Some(ref token)) =
                (&config.plivo_auth_id, &config.plivo_auth_token)
            {
                router = router.add_sms(Arc::new(PlivoSmsSender::new(
                    id.clone(),
                    token.clone(),
                    config.plivo_from.clone(),
                )));
            }
        }
        "email" => {
            if let (Some(ref api_key), Some(ref from)) =
                (&config.resend_api_key, &config.from_email)
            {
                router = router.add_email(Arc::new(ResendEmailSender::new(
                    api_key.clone(),
                    from.clone(),
                )));
            }
            if let (Some(ref ak), Some(ref sk), Some(ref region), Some(ref from)) = (
                &config.ses_access_key,
                &config.ses_secret_key,
                &config.ses_region,
                &config.from_email,
            ) {
                let sender =
                    SesEmailSender::new(ak.clone(), sk.clone(), region.clone(), from.clone())?;
                router = router.add_email(Arc::new(sender));
            }
            if let (Some(ref host), Some(port), Some(ref user), Some(ref pass), Some(ref from)) = (
                &config.smtp_host,
                config.smtp_port,
                &config.smtp_username,
                &config.smtp_password,
                &config.from_email,
            ) {
                let sender = SmtpEmailSender::new(
                    host.clone(),
                    port,
                    user.clone(),
                    pass.clone(),
                    from.clone(),
                )?;
                router = router.add_email(Arc::new(sender));
            }
        }
        _ => {}
    }

    Ok(router)
}

/// Add a single provider to a router based on provider name and credentials JSON.
fn add_provider_to_router(
    router: WaterfallRouter,
    provider: &str,
    channel: &str,
    creds: &serde_json::Value,
) -> anyhow::Result<WaterfallRouter> {
    match (channel, provider) {
        ("sms", "telnyx") => {
            let api_key = creds["api_key"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().map(String::from);
            Ok(router.add_sms(Arc::new(TelnyxSmsSender::new(api_key, from))))
        }
        ("sms", "twilio") => {
            let sid = creds["account_sid"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let token = creds["auth_token"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().map(String::from);
            Ok(router.add_sms(Arc::new(TwilioSmsSender::new(sid, token, from))))
        }
        ("sms", "plivo") => {
            let id = creds["auth_id"].as_str().unwrap_or_default().to_string();
            let token = creds["auth_token"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().map(String::from);
            Ok(router.add_sms(Arc::new(PlivoSmsSender::new(id, token, from))))
        }
        ("email", "resend") => {
            let api_key = creds["api_key"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().unwrap_or_default().to_string();
            Ok(router.add_email(Arc::new(ResendEmailSender::new(api_key, from))))
        }
        ("email", "ses") => {
            let ak = creds["access_key"].as_str().unwrap_or_default().to_string();
            let sk = creds["secret_key"].as_str().unwrap_or_default().to_string();
            let region = creds["region"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().unwrap_or_default().to_string();
            let sender = SesEmailSender::new(ak, sk, region, from)?;
            Ok(router.add_email(Arc::new(sender)))
        }
        ("email", "smtp") => {
            let host = creds["host"].as_str().unwrap_or_default().to_string();
            let port = creds["port"].as_u64().unwrap_or(587) as u16;
            let user = creds["username"].as_str().unwrap_or_default().to_string();
            let pass = creds["password"].as_str().unwrap_or_default().to_string();
            let from = creds["from"].as_str().unwrap_or_default().to_string();
            let sender = SmtpEmailSender::new(host, port, user, pass, from)?;
            Ok(router.add_email(Arc::new(sender)))
        }
        _ => anyhow::bail!("unknown provider: {channel}/{provider}"),
    }
}
