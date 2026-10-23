mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use common::FakeStore;
use isolate::content::ContentResolver;
use isolate::server::{router, router_with_budget, BUCKET_REQUEST_BUDGET};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

fn test_resolver(store: &FakeStore) -> ContentResolver {
    ContentResolver::new(
        Arc::new(store.clone()),
        Arc::new(|| Duration::ZERO),
        Duration::from_secs(10),
    )
}

async fn response(
    app: axum::Router,
    method: &str,
    target: &str,
    host: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(target)
            .header("host", host)
            .body(Body::from("body is never read"))
            .expect("request"),
    )
    .await
    .expect("infallible router")
}

#[tokio::test]
async fn exact_get_and_head_logo_requests_serve_validated_bytes() {
    for (name, media_type) in [
        ("logo.png", "image/png"),
        ("logo.webp", "image/webp"),
        ("logo.jpg", "image/jpeg"),
    ] {
        let store = FakeStore::default();
        store.object(
            "sites/example.com/branding.json",
            format!(r#"{{"logo":"{name}","unknown":true}}"#),
        );
        store.object(
            &format!("sites/example.com/{name}"),
            format!("{name} test bytes"),
        );
        for method in ["GET", "HEAD"] {
            let response = response(
                router(test_resolver(&store)),
                method,
                "/__isolate/logo",
                "example.com",
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{method} {name}");
            assert_eq!(response.headers()["content-type"], media_type);
            assert_eq!(
                response.headers()["cache-control"],
                "no-store, no-transform"
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            if method == "HEAD" {
                assert!(body.is_empty());
            } else {
                assert_eq!(body, format!("{name} test bytes"));
            }
        }
    }
}

#[tokio::test]
async fn every_other_representative_request_gets_the_frozen_page_response() {
    let store = FakeStore::default();
    store.object(
        "sites/example.com/message.txt",
        "Method and route test message",
    );
    store.object("sites/example.com/branding.json", br#"{"logo":"logo.png"}"#);
    store.object("sites/example.com/logo.png", b"png bytes".to_vec());

    for (method, target) in [
        ("GET", "/"),
        ("HEAD", "/status"),
        ("POST", "/api/orders?attempt=2"),
        ("DELETE", "/.env"),
        ("PROPFIND", "/deep/path"),
        ("POST", "/__isolate/logo"),
        ("GET", "/__isolate/logo?download=1"),
        ("GET", "/__isolate/logo/"),
        ("GET", "/__isolate%2Flogo"),
    ] {
        let response = response(router(test_resolver(&store)), method, target, "example.com").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        for (name, value) in [
            ("content-type", "text/html; charset=utf-8"),
            ("cache-control", "no-store, no-transform"),
            ("retry-after", "3600"),
            ("x-robots-tag", "noindex"),
        ] {
            assert_eq!(response.headers()[name], value, "{method} {target}");
        }
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        if method == "HEAD" {
            assert!(body.is_empty());
        } else {
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(body.contains(r#"data-testid="isolate-page""#));
            assert!(body.contains("Method and route test message"));
        }
    }
}

#[tokio::test]
async fn invalid_or_missing_logo_falls_back_to_a_503_page() {
    for branding in [r#"{"logo":"logo.svg"}"#, r#"{"logo":"logo.png"}"#] {
        let store = FakeStore::default();
        store.object(
            "sites/example.com/message.txt",
            "Missing logo fallback message",
        );
        store.object("sites/example.com/branding.json", branding);
        let response = response(
            router(test_resolver(&store)),
            "GET",
            "/__isolate/logo",
            "example.com",
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8(body.to_vec())
            .unwrap()
            .contains("Missing logo fallback message"));
    }
}

#[tokio::test]
async fn malformed_branding_keeps_the_503_and_built_in_branding() {
    let store = FakeStore::default();
    store.object(
        "sites/example.com/message.txt",
        "Malformed branding test message",
    );
    store.object("sites/example.com/branding.json", br#"{"site_name":"#);
    let response = response(
        router(test_resolver(&store)),
        "GET",
        "/maintenance",
        "example.com",
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("Malformed branding test message"));
    assert!(body.contains("<h1>Temporarily unavailable</h1>"));
}

#[tokio::test]
async fn one_short_budget_cancels_a_blocked_store_and_serves_built_ins() {
    let store = FakeStore::default();
    store.block();
    let budget = Duration::from_millis(25);
    let started = Instant::now();
    let response = response(
        router_with_budget(test_resolver(&store), budget),
        "GET",
        "/",
        "blocked.example",
    )
    .await;
    let elapsed = started.elapsed();

    assert!(elapsed < budget * 3, "blocked request took {elapsed:?}");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8(body.to_vec())
        .unwrap()
        .contains("This site is temporarily unavailable"));
    assert_eq!(store.calls("sites/blocked.example/branding.json"), 1);
    assert_eq!(store.calls("default/branding.json"), 0);
}

#[tokio::test]
async fn a_deadline_after_a_good_read_serves_the_validated_last_good_content() {
    let store = FakeStore::default();
    store.object("sites/example.com/message.txt", "Last good blocked message");
    store.object(
        "sites/example.com/branding.json",
        br##"{"site_name":"Last good site","primary_color":"#123456"}"##,
    );
    let resolver = ContentResolver::new(
        Arc::new(store.clone()),
        Arc::new(|| Duration::ZERO),
        Duration::ZERO,
    );
    let primed = resolver.resolve("example.com").await;
    assert_eq!(primed.message, "Last good blocked message");
    store.block();

    let budget = Duration::from_millis(25);
    let started = Instant::now();
    let response = response(
        router_with_budget(resolver, budget),
        "GET",
        "/",
        "example.com",
    )
    .await;
    assert!(started.elapsed() < budget * 3);
    let body = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("Last good blocked message"));
    assert!(body.contains("<h1>Last good site</h1>"));
    assert!(body.contains("color: #123456;"));
}

#[test]
fn production_bucket_budget_is_exactly_two_seconds() {
    assert_eq!(BUCKET_REQUEST_BUDGET, Duration::from_secs(2));
}

#[tokio::test]
async fn it_answers_on_the_ipv6_wildcard() {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0u16; 8], 0)))
        .await
        .expect("bind IPv6 wildcard");
    let port = listener.local_addr().unwrap().port();
    let store = FakeStore::default();
    store.object("sites/probe.example/message.txt", "Probe message");
    tokio::spawn(async move {
        axum::serve(listener, router(test_resolver(&store)))
            .await
            .ok();
    });

    let mut stream = tokio::net::TcpStream::connect(("::1", port)).await.unwrap();
    stream
        .write_all(b"GET /anything HTTP/1.1\r\nHost: probe.example\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let response = String::from_utf8_lossy(&raw);
    assert!(response.starts_with("HTTP/1.1 503 Service Unavailable"));
    assert!(response.contains("Probe message"));
}
