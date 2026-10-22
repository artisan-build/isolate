//! The page, the escaping and the host-to-message configuration.

use isolate::page::{
    escape_html, render_page, Config, BUILTIN_DEFAULT_MESSAGE, MESSAGE_PLACEHOLDER, PAGE_TEMPLATE,
};
use sha2::{Digest, Sha256};

/// Pins `templates/isolated.html`. The `main` branch's Go tests assert the same
/// constant, which is what keeps the page byte-identical across the two
/// runtimes. Changing the template means changing it on both branches.
const TEMPLATE_SHA256: &str = "34cd895cbe4d1429fc0bbcc12f759d1d0331e34f7c48fca3fac604848d75e1eb";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn template_hash_matches_the_other_runtime() {
    let digest = Sha256::digest(PAGE_TEMPLATE.as_bytes());

    assert_eq!(
        hex(&digest),
        TEMPLATE_SHA256,
        "templates/isolated.html has drifted from the copy the main branch pins"
    );
}

#[test]
fn host_selects_the_message() {
    let config = Config::load(
        r#"{"one.example":"One is isolated.","two.example":"Two is isolated."}"#,
        "Everything else is isolated.",
    );

    for (host, expected) in [
        ("one.example", "One is isolated."),
        ("TWO.Example:8080", "Two is isolated."),
        ("one.example.", "One is isolated."),
        ("three.example", "Everything else is isolated."),
        ("", "Everything else is isolated."),
    ] {
        assert_eq!(config.message_for(host), expected, "host {host:?}");
    }
}

#[test]
fn an_unknown_host_falls_back_to_the_built_in_default() {
    let config = Config::load("", "");

    assert_eq!(
        config.message_for("anything.example"),
        BUILTIN_DEFAULT_MESSAGE
    );
}

#[test]
fn malformed_messages_fall_back_without_panicking() {
    for value in [
        r#"{"example.com": }"#,
        "not json at all",
        r#"["example.com","a message"]"#,
        r#"{"example.com": 42}"#,
        "{",
        "null",
    ] {
        let config = Config::load(value, "The configured default.");

        assert_eq!(
            config.host_count(),
            0,
            "ISOLATE_MESSAGES={value:?} was parsed"
        );
        assert_eq!(
            config.message_for("example.com"),
            "The configured default.",
            "ISOLATE_MESSAGES={value:?}"
        );
    }
}

#[test]
fn messages_are_html_escaped() {
    let raw = r#"<script>alert("x" & 'y')</script>"#;
    let config = Config::load(
        r#"{"xss.example":"<script>alert(\"x\" & 'y')</script>"}"#,
        "",
    );

    assert_eq!(config.message_for("xss.example"), raw);

    let body = render_page(PAGE_TEMPLATE, config.message_for("xss.example"));
    let escaped = "&lt;script&gt;alert(&quot;x&quot; &amp; &#39;y&#39;)&lt;/script&gt;";

    assert!(
        body.contains(&format!("<p>{escaped}</p>")),
        "message was not escaped"
    );
    assert!(
        !body.contains("<script>"),
        "an unescaped <script> tag survived"
    );
}

#[test]
fn escape_html_covers_the_five_characters() {
    for (input, expected) in [
        ("", ""),
        ("plain", "plain"),
        ("&", "&amp;"),
        ("<>", "&lt;&gt;"),
        ("\"'", "&quot;&#39;"),
        ("café — ok", "café — ok"),
        ("a&<b", "a&amp;&lt;b"),
    ] {
        assert_eq!(escape_html(input), expected, "input {input:?}");
    }
}

#[test]
fn rendering_leaves_no_placeholder() {
    let body = render_page(PAGE_TEMPLATE, "A message.");

    assert!(
        !body.contains(MESSAGE_PLACEHOLDER),
        "the placeholder survived"
    );
    assert!(body.contains("<p>A message.</p>"), "the message is missing");
}
