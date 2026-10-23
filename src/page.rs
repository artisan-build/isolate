//! Rendering for the shared isolation page.

use crate::content::PageContent;

pub const PAGE_TEMPLATE: &str = include_str!("../templates/isolated.html");

const MESSAGE_PLACEHOLDER: &str = "{{MESSAGE}}";
const SITE_NAME_PLACEHOLDER: &str = "{{SITE_NAME}}";
const PRIMARY_COLOR_PLACEHOLDER: &str = "{{PRIMARY_COLOR}}";
const BACKGROUND_COLOR_PLACEHOLDER: &str = "{{BACKGROUND_COLOR}}";
const TEXT_COLOR_PLACEHOLDER: &str = "{{TEXT_COLOR}}";
const LOGO_PLACEHOLDER: &str = "{{LOGO}}";
const LOGO_MARKUP: &str = r#"<img class="logo" src="/__isolate/logo" alt="">"#;

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

pub fn render_paragraphs(message: &str) -> String {
    let normalized = message.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs = Vec::new();
    let mut lines = Vec::new();

    for line in normalized.split('\n') {
        if line.trim().is_empty() {
            if !lines.is_empty() {
                paragraphs.push(format!("<p>{}</p>", escape_html(&lines.join("\n"))));
                lines.clear();
            }
        } else {
            lines.push(line);
        }
    }
    if !lines.is_empty() {
        paragraphs.push(format!("<p>{}</p>", escape_html(&lines.join("\n"))));
    }

    paragraphs.join("\n")
}

/// Replaces placeholders while scanning only the template. Placeholder-looking
/// text supplied by the bucket is therefore never expanded a second time.
pub fn render_page(template: &str, content: &PageContent) -> String {
    let site_name = escape_html(&content.site_name);
    let message = render_paragraphs(&content.message);
    let logo = if content.logo.is_some() {
        LOGO_MARKUP
    } else {
        ""
    };
    let replacements = [
        (SITE_NAME_PLACEHOLDER, site_name.as_str()),
        (PRIMARY_COLOR_PLACEHOLDER, content.primary_color.as_str()),
        (
            BACKGROUND_COLOR_PLACEHOLDER,
            content.background_color.as_str(),
        ),
        (TEXT_COLOR_PLACEHOLDER, content.text_color.as_str()),
        (LOGO_PLACEHOLDER, logo),
        (MESSAGE_PLACEHOLDER, message.as_str()),
    ];

    let mut rendered = String::with_capacity(template.len() + message.len() + site_name.len());
    let mut remaining = template;
    while !remaining.is_empty() {
        let next = replacements
            .iter()
            .filter_map(|(placeholder, value)| {
                remaining
                    .find(placeholder)
                    .map(|offset| (offset, *placeholder, *value))
            })
            .min_by_key(|(offset, _, _)| *offset);

        let Some((offset, placeholder, value)) = next else {
            rendered.push_str(remaining);
            break;
        };
        rendered.push_str(&remaining[..offset]);
        rendered.push_str(value);
        remaining = &remaining[offset + placeholder.len()..];
    }

    rendered
}
