use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "password-reset".into(),
        name: "Password Reset".into(),
        subject: "Reset your {{ app_name }} password".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Password Reset</h2>
<p>We received a request to reset your {{ app_name }} password.</p>
<p style="margin:24px 0;"><a href="{{ reset_url }}" style="background:#111;color:#fff;padding:12px 24px;text-decoration:none;border-radius:4px;display:inline-block;">Reset Password</a></p>
<p style="color:#666;">This link expires in {{ expiry }}.</p>
<p style="color:#999;font-size:12px;margin-top:32px;">If you did not request a password reset, please ignore this email.</p>
</div>"#
            .into(),
        text_body: "Reset your {{ app_name }} password\n\nVisit this link to reset your password: {{ reset_url }}\n\nExpires in {{ expiry }}.\n\nIf you did not request this, please ignore this email.".into(),
        variables: vec!["reset_url".into(), "app_name".into(), "expiry".into()],
    }
}
