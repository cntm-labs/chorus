use crate::template::Template;

pub fn template() -> Template {
    Template {
        slug: "welcome".into(),
        name: "Welcome".into(),
        subject: "Welcome to {{ app_name }}".into(),
        html_body: r#"<div style="max-width:480px;margin:0 auto;font-family:sans-serif;padding:24px;">
<h2 style="margin:0 0 16px;">Welcome{% if user_name %}, {{ user_name }}{% endif %}!</h2>
<p>Thanks for joining {{ app_name }}. We're glad to have you.</p>
</div>"#
            .into(),
        text_body: "Welcome{% if user_name %}, {{ user_name }}{% endif %}!\n\nThanks for joining {{ app_name }}. We're glad to have you.".into(),
        variables: vec!["user_name".into(), "app_name".into()],
    }
}
