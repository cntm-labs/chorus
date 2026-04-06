mod email_verify;
mod magic_link;
mod otp;
mod password_reset;
mod welcome;

use crate::template::Template;

/// Returns all built-in auth templates.
pub fn builtin_templates() -> Vec<Template> {
    vec![
        otp::template(),
        password_reset::template(),
        magic_link::template(),
        email_verify::template(),
        welcome::template(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn builtin_templates_returns_five() {
        let templates = builtin_templates();
        assert_eq!(templates.len(), 5);
    }

    #[test]
    fn builtin_templates_have_expected_slugs() {
        let templates = builtin_templates();
        let slugs: Vec<&str> = templates.iter().map(|t| t.slug.as_str()).collect();
        assert!(slugs.contains(&"otp"));
        assert!(slugs.contains(&"password-reset"));
        assert!(slugs.contains(&"magic-link"));
        assert!(slugs.contains(&"email-verify"));
        assert!(slugs.contains(&"welcome"));
    }

    #[test]
    fn otp_template_renders() {
        let t = otp::template();
        let vars = HashMap::from([
            ("code".into(), "123456".into()),
            ("app_name".into(), "TestApp".into()),
            ("expiry".into(), "10 minutes".into()),
        ]);
        let r = t.render(&vars).unwrap();
        assert!(r.subject.contains("TestApp"));
        assert!(r.html_body.contains("123456"));
        assert!(r.text_body.contains("10 minutes"));
    }

    #[test]
    fn password_reset_template_renders() {
        let t = password_reset::template();
        let vars = HashMap::from([
            ("reset_url".into(), "https://example.com/reset".into()),
            ("app_name".into(), "TestApp".into()),
            ("expiry".into(), "1 hour".into()),
        ]);
        let r = t.render(&vars).unwrap();
        assert!(r.subject.contains("TestApp"));
        assert!(r.html_body.contains("https://example.com/reset"));
        assert!(r.text_body.contains("1 hour"));
    }

    #[test]
    fn magic_link_template_renders() {
        let t = magic_link::template();
        let vars = HashMap::from([
            ("magic_url".into(), "https://example.com/magic".into()),
            ("app_name".into(), "TestApp".into()),
            ("expiry".into(), "15 minutes".into()),
        ]);
        let r = t.render(&vars).unwrap();
        assert!(r.subject.contains("TestApp"));
        assert!(r.html_body.contains("https://example.com/magic"));
    }

    #[test]
    fn email_verify_template_renders() {
        let t = email_verify::template();
        let vars = HashMap::from([
            ("verify_url".into(), "https://example.com/verify".into()),
            ("app_name".into(), "TestApp".into()),
        ]);
        let r = t.render(&vars).unwrap();
        assert!(r.subject.contains("TestApp"));
        assert!(r.html_body.contains("https://example.com/verify"));
    }

    #[test]
    fn welcome_template_renders_with_name() {
        let t = welcome::template();
        let vars = HashMap::from([
            ("user_name".into(), "Alice".into()),
            ("app_name".into(), "TestApp".into()),
        ]);
        let r = t.render(&vars).unwrap();
        assert!(r.subject.contains("TestApp"));
        assert!(r.html_body.contains("Alice"));
    }

    #[test]
    fn welcome_template_renders_without_name() {
        let t = welcome::template();
        let vars = HashMap::from([("app_name".into(), "TestApp".into())]);
        let r = t.render(&vars).unwrap();
        assert!(r.html_body.contains("Welcome!"));
        assert!(!r.html_body.contains("Alice"));
    }
}
