use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "email-verify".into(),
        name: "Email Verification".into(),
        subject: "Verify your email for {{ app_name }}".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Verify Your Email</h2>
<p>Please verify your email address for {{ app_name }}.</p>
<p style="margin:24px 0;"><a href="{{ verify_url }}" style="background:#111;color:#fff;padding:12px 24px;text-decoration:none;border-radius:4px;display:inline-block;">Verify Email</a></p>
<p style="color:#999;font-size:12px;margin-top:32px;">If you did not create an account, please ignore this email.</p>
</div>"#
            .into(),
        text_body: "Verify your email for {{ app_name }}\n\nVisit this link to verify: {{ verify_url }}\n\nIf you did not create an account, please ignore this email.".into(),
        variables: vec!["verify_url".into(), "app_name".into()],
    }
}
