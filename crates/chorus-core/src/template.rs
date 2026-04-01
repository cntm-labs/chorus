use crate::error::ChorusError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub slug: String,
    pub name: String,
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
    pub variables: Vec<String>,
}

impl Template {
    /// Render template by replacing {{variable}} placeholders with values.
    pub fn render(
        &self,
        variables: &HashMap<String, String>,
    ) -> Result<RenderedTemplate, ChorusError> {
        let subject = Self::replace_vars(&self.subject, variables);
        let html_body = Self::replace_vars(&self.html_body, variables);
        let text_body = Self::replace_vars(&self.text_body, variables);

        Ok(RenderedTemplate {
            subject,
            html_body,
            text_body,
        })
    }

    fn replace_vars(text: &str, variables: &HashMap<String, String>) -> String {
        let mut result = text.to_string();
        for (key, value) in variables {
            result = result.replace(&format!("{{{{{}}}}}", key), value);
        }
        result
    }
}

#[derive(Debug, Clone)]
pub struct RenderedTemplate {
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_template() -> Template {
        Template {
            slug: "otp".to_string(),
            name: "OTP Email".to_string(),
            subject: "Your {{app_name}} code".to_string(),
            html_body: "<p>Code: <strong>{{code}}</strong>. Expires in {{expire}} min.</p>"
                .to_string(),
            text_body: "Code: {{code}}. Expires in {{expire}} min.".to_string(),
            variables: vec![
                "code".to_string(),
                "app_name".to_string(),
                "expire".to_string(),
            ],
        }
    }

    #[test]
    fn render_replaces_all_variables() {
        let tmpl = test_template();
        let mut vars = HashMap::new();
        vars.insert("code".to_string(), "123456".to_string());
        vars.insert("app_name".to_string(), "Orbit".to_string());
        vars.insert("expire".to_string(), "5".to_string());

        let rendered = tmpl.render(&vars).unwrap();
        assert_eq!(rendered.subject, "Your Orbit code");
        assert!(rendered.html_body.contains("<strong>123456</strong>"));
        assert!(rendered.text_body.contains("123456"));
        assert!(rendered.text_body.contains("5 min"));
    }

    #[test]
    fn render_leaves_unknown_vars_as_is() {
        let tmpl = test_template();
        let vars = HashMap::new();
        let rendered = tmpl.render(&vars).unwrap();
        assert!(rendered.subject.contains("{{app_name}}"));
    }

    #[test]
    fn render_handles_repeated_variable() {
        let tmpl = Template {
            slug: "test".into(),
            name: "Test".into(),
            subject: "{{code}} is your code {{code}}".into(),
            html_body: "".into(),
            text_body: "".into(),
            variables: vec!["code".into()],
        };
        let mut vars = HashMap::new();
        vars.insert("code".into(), "999".into());
        let rendered = tmpl.render(&vars).unwrap();
        assert_eq!(rendered.subject, "999 is your code 999");
    }
}
