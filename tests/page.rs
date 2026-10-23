use isolate::content::{PageContent, ResolvedLogo};
use isolate::page::{escape_html, render_page, render_paragraphs, PAGE_TEMPLATE};
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn pr1_handoff_artifacts_are_byte_identical() {
    for (bytes, expected, name) in [
        (
            include_bytes!("../docs/content-contract.md").as_slice(),
            "74b26d959052b3df343d8cd88377731b3c41acc5b2f945e96bbeda480c05bc75",
            "content contract",
        ),
        (
            include_bytes!("../templates/isolated.html").as_slice(),
            "5e15e15b0fca3e57056e5d83cbc927300b02dc412728a1083e810e0b5d8f4815",
            "template",
        ),
        (
            include_bytes!("../testdata/builtin.html").as_slice(),
            "2d7d4c39a0bf5d90e6479c9a1bfba23b56b36a909abdcca3f733613a11d1524b",
            "built-in golden body",
        ),
        (
            include_bytes!("../testdata/customized.html").as_slice(),
            "1ef9044cb47f1866b73e52323ff091484217eaddaf0d72483830dc6620c61b43",
            "customized golden body",
        ),
    ] {
        assert_eq!(digest(bytes), expected, "{name} differs from PR1 cafe1e6b");
    }
}

#[test]
fn rendering_escapes_content_splits_paragraphs_and_substitutes_once() {
    let content = PageContent {
        message: "First <unsafe> paragraph\r\n\r\nSecond & {{SITE_NAME}}".to_owned(),
        site_name: r#"A "quoted" <site> {{MESSAGE}}"#.to_owned(),
        primary_color: "#123456".to_owned(),
        background_color: "#abcdef".to_owned(),
        text_color: "#654321".to_owned(),
        logo: None,
    };
    let body = render_page(PAGE_TEMPLATE, &content);

    for marker in [
        r#"data-testid="isolate-page""#,
        r#"data-testid="isolate-brand""#,
        r#"data-testid="isolate-message""#,
        r#"data-testid="isolate-footer""#,
    ] {
        assert!(body.contains(marker), "missing structural marker {marker}");
    }
    assert!(body.contains("<h1>A &quot;quoted&quot; &lt;site&gt; {{MESSAGE}}</h1>"));
    assert!(
        body.contains("<p>First &lt;unsafe&gt; paragraph</p>\n<p>Second &amp; {{SITE_NAME}}</p>")
    );
    assert!(body.contains("background: #abcdef;"));
    assert!(body.contains("color: #654321;"));
    assert!(body.contains("color: #123456;"));
}

#[test]
fn escaping_and_blank_line_rules_use_test_created_text() {
    assert_eq!(escape_html(r#"<&>"'"#), "&lt;&amp;&gt;&quot;&#39;");
    assert_eq!(
        render_paragraphs("alpha\nline two\n \t\nbeta\r\n\r\ngamma"),
        "<p>alpha\nline two</p>\n<p>beta</p>\n<p>gamma</p>"
    );
}

#[test]
fn complete_bodies_match_the_copied_golden_fixtures() {
    let built_in = PageContent::built_in();
    let customized = PageContent {
        message: "Scheduled maintenance is underway.\n\nStatus updates are posted separately."
            .to_owned(),
        site_name: "Example & Company".to_owned(),
        primary_color: "#1d4ed8".to_owned(),
        background_color: "#fff7ed".to_owned(),
        text_color: "#0f172a".to_owned(),
        logo: Some(ResolvedLogo {
            bytes: b"fixture-logo".to_vec(),
            media_type: "image/webp",
        }),
    };

    assert_eq!(
        render_page(PAGE_TEMPLATE, &built_in).as_bytes(),
        include_bytes!("../testdata/builtin.html")
    );
    assert_eq!(
        render_page(PAGE_TEMPLATE, &customized).as_bytes(),
        include_bytes!("../testdata/customized.html")
    );
}
