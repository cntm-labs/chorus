use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "otp".into(),
        name: "OTP Verification Code".into(),
        subject: "Your {{ app_name }} verification code".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Verification Code</h2>
<p>Your {{ app_name }} code is:</p>
<p style="font-size:32px;letter-spacing:8px;font-weight:bold;margin:24px 0;">{{ code }}</p>
<p style="color:#666;">This code expires in {{ expiry }}.</p>
<p style="color:#999;font-size:12px;margin-top:32px;">If you did not request this code, please ignore this email.</p>
</div>"#
            .into(),
        text_body: "Your {{ app_name }} verification code is: {{ code }}\n\nExpires in {{ expiry }}.\n\nIf you did not request this code, please ignore this email.".into(),
        variables: vec!["code".into(), "app_name".into(), "expiry".into()],
    }
}
