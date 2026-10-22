//! The isolation page and the configuration that fills in its one message.

use std::collections::HashMap;

/// The last fallback: used when neither `ISOLATE_MESSAGES` nor
/// `ISOLATE_DEFAULT_MESSAGE` supplies a message.
pub const BUILTIN_DEFAULT_MESSAGE: &str =
    "This site is temporarily unavailable. It will be back shortly.";

/// The only substitution the shared template carries.
pub const MESSAGE_PLACEHOLDER: &str = "{{MESSAGE}}";

/// The shared isolation page, byte-identical to the copy on the `main` branch.
/// `TEMPLATE_SHA256` in `tests/page.rs` pins it.
pub const PAGE_TEMPLATE: &str = include_str!("../templates/isolated.html");

/// Boot-time configuration: a message per host plus a fallback.
#[derive(Debug, Clone)]
pub struct Config {
    messages: HashMap<String, String>,
    fallback: String,
}

impl Config {
    /// Resolves the configuration from raw environment values. A malformed
    /// `messages_json` is logged and ignored, never fatal.
    pub fn load(messages_json: &str, default_message: &str) -> Self {
        let fallback = match default_message.trim() {
            "" => BUILTIN_DEFAULT_MESSAGE.to_owned(),
            trimmed => trimmed.to_owned(),
        };

        let mut messages = HashMap::new();
        let raw = messages_json.trim();

        if !raw.is_empty() {
            match serde_json::from_str::<HashMap<String, String>>(raw) {
                Ok(parsed) => {
                    for (host, message) in parsed {
                        messages.insert(normalize_host(&host), message);
                    }
                }
                Err(error) => {
                    eprintln!("isolate: ignoring malformed ISOLATE_MESSAGES: {error}");
                }
            }
        }

        Self { messages, fallback }
    }

    /// Picks the message for a `Host` header value.
    pub fn message_for(&self, host: &str) -> &str {
        self.messages
            .get(&normalize_host(host))
            .map_or(self.fallback.as_str(), String::as_str)
    }

    /// How many per-host messages were accepted. Lets a test tell "parsed and
    /// empty" apart from "rejected as malformed".
    pub fn host_count(&self) -> usize {
        self.messages.len()
    }
}

/// Lowercases a host, drops any port and any trailing root dot, so that
/// `Example.COM:8080` and `example.com.` share one key.
fn normalize_host(host: &str) -> String {
    let host = host.trim().to_lowercase();

    let without_port = if let Some(rest) = host.strip_prefix('[') {
        match rest.find(']') {
            Some(end) => &rest[..end],
            None => rest,
        }
    } else {
        match host.rfind(':') {
            Some(colon) => &host[..colon],
            None => host.as_str(),
        }
    };

    without_port
        .strip_suffix('.')
        .unwrap_or(without_port)
        .to_owned()
}

/// Escapes the five characters that matter in element text. Hand-rolled, rather
/// than taken from a crate, so that the Go and Rust runtimes emit byte-identical
/// bodies.
pub fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }

    escaped
}

/// Substitutes the escaped message into the shared template.
pub fn render_page(template: &str, message: &str) -> String {
    template.replace(MESSAGE_PLACEHOLDER, &escape_html(message))
}
