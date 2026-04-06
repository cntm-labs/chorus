use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "magic-link".into(),
        name: "Magic Link Sign-in".into(),
        subject: "Sign in to {{ app_name }}".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Sign In</h2>
<p>Click the button below to sign in to {{ app_name }}.</p>
<p style="margin:24px 0;"><a href="{{ magic_url }}" style="background:#111;color:#fff;padding:12px 24px;text-decoration:none;border-radius:4px;display:inline-block;">Sign In</a></p>
<p style="color:#666;">This link expires in {{ expiry }}.</p>
<p style="color:#999;font-size:12px;margin-top:32px;">If you did not request this link, please ignore this email.</p>
</div>"#
            .into(),
        text_body: "Sign in to {{ app_name }}\n\nVisit this link to sign in: {{ magic_url }}\n\nExpires in {{ expiry }}.\n\nIf you did not request this, please ignore this email.".into(),
        variables: vec!["magic_url".into(), "app_name".into(), "expiry".into()],
    }
}
