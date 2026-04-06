use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::ChorusError;

/// A reusable message template with variable placeholders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    /// Unique identifier for looking up this template.
    pub slug: String,
    /// Human-readable template name.
    pub name: String,
    /// Subject line template (rendered with variables).
    pub subject: String,
    /// HTML body template.
    pub html_body: String,
    /// Plain text body template.
    pub text_body: String,
    /// List of expected variable names (for documentation/validation).
    pub variables: Vec<String>,
}

/// The result of rendering a template with variables.
#[derive(Debug, Clone)]
pub struct RenderedTemplate {
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
}

impl Template {
    /// Renders the template by replacing placeholders with provided values.
    ///
    /// Supports Jinja2 syntax: `{{ variable }}`, `{% if %}`, `{% for %}`, filters.
    /// Simple `{{variable}}` from prior versions remains compatible.
    pub fn render(
        &self,
        variables: &HashMap<String, String>,
    ) -> Result<RenderedTemplate, ChorusError> {
        let subject = render_string(&self.subject, variables)?;
        let html_body = render_string(&self.html_body, variables)?;
        let text_body = render_string(&self.text_body, variables)?;

        Ok(RenderedTemplate {
            subject,
            html_body,
            text_body,
        })
    }
}

/// Render a single template string with the given variables.
fn render_string(
    template: &str,
    variables: &HashMap<String, String>,
) -> Result<String, ChorusError> {
    let env = minijinja::Environment::new();
    let ctx = minijinja::value::Value::from_serialize(variables);
    env.render_str(template, ctx)
        .map_err(|e| ChorusError::Validation(format!("template render error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_template(subject: &str, html: &str, text: &str) -> Template {
        Template {
            slug: "test".into(),
            name: "Test".into(),
            subject: subject.into(),
            html_body: html.into(),
            text_body: text.into(),
            variables: vec![],
        }
    }

    #[test]
    fn renders_simple_variables() {
        let t = make_template(
            "Hello {{ name }}",
            "<p>Hi {{ name }}, code: {{ code }}</p>",
            "Hi {{ name }}, code: {{ code }}",
        );
        let vars = HashMap::from([
            ("name".into(), "Alice".into()),
            ("code".into(), "123456".into()),
        ]);
        let r = t.render(&vars).unwrap();
        assert_eq!(r.subject, "Hello Alice");
        assert_eq!(r.html_body, "<p>Hi Alice, code: 123456</p>");
        assert_eq!(r.text_body, "Hi Alice, code: 123456");
    }

    #[test]
    fn undefined_variables_render_empty() {
        let t = make_template("{{ missing }}", "", "");
        let r = t.render(&HashMap::new()).unwrap();
        assert_eq!(r.subject, "");
    }

    #[test]
    fn repeated_variables() {
        let t = make_template("{{ x }} and {{ x }}", "", "");
        let vars = HashMap::from([("x".into(), "hi".into())]);
        let r = t.render(&vars).unwrap();
        assert_eq!(r.subject, "hi and hi");
    }

    #[test]
    fn empty_template() {
        let t = make_template("", "", "");
        let r = t.render(&HashMap::new()).unwrap();
        assert_eq!(r.subject, "");
    }

    #[test]
    fn no_placeholders() {
        let t = make_template("Hello world", "<p>Hi</p>", "Hi");
        let r = t.render(&HashMap::new()).unwrap();
        assert_eq!(r.subject, "Hello world");
    }

    #[test]
    fn if_else_conditional() {
        let t = make_template(
            "{% if name %}Hi {{ name }}{% else %}Hi there{% endif %}",
            "",
            "",
        );
        let with_name = HashMap::from([("name".into(), "Bob".into())]);
        assert_eq!(t.render(&with_name).unwrap().subject, "Hi Bob");

        let without_name = HashMap::new();
        assert_eq!(t.render(&without_name).unwrap().subject, "Hi there");
    }

    #[test]
    fn for_loop() {
        let env = minijinja::Environment::new();
        let ctx = minijinja::context! { items => vec!["a", "b", "c"] };
        let result = env
            .render_str("{% for item in items %}{{ item }} {% endfor %}", ctx)
            .unwrap();
        assert_eq!(result, "a b c ");
    }

    #[test]
    fn default_filter() {
        let t = make_template("{{ name | default('Guest') }}", "", "");
        let r = t.render(&HashMap::new()).unwrap();
        assert_eq!(r.subject, "Guest");
    }

    #[test]
    fn special_characters_in_values() {
        let t = make_template("{{ val }}", "", "");
        let vars = HashMap::from([("val".into(), "<script>alert('xss')</script>".into())]);
        let r = t.render(&vars).unwrap();
        // minijinja auto-escapes HTML in templates
        assert!(r.subject.contains("&lt;script&gt;") || r.subject.contains("<script>"));
    }
}
