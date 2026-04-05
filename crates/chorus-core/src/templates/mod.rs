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
