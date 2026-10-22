//! Every request that reaches the process gets the same page back.

use std::net::SocketAddr;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use isolate::page::{render_page, Config, PAGE_TEMPLATE};
use isolate::server::router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const METHODS: [&str; 8] = [
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "PROPFIND",
];

const PATHS: [&str; 6] = [
    "/",
    "/wp-admin/x.php?a=1",
    "/api/v1/orders",
    "/.env",
    "/a/very/deep/path?q=1&q=2",
    "/%2e%2e/etc/passwd",
];

#[tokio::test]
async fn every_method_path_and_query_gets_the_page() {
    let expected = render_page(PAGE_TEMPLATE, "Example is isolated.");

    for method in METHODS {
        for path in PATHS {
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header("host", "example.com")
                .body(Body::from("a body that must never be read"))
                .expect("build the request");

            let response = router(Config::load(
                r#"{"example.com":"Example is isolated."}"#,
                "",
            ))
            .oneshot(request)
            .await
            .expect("the router is infallible");

            let context = format!("{method} {path}");

            assert_eq!(
                response.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{context}"
            );

            for (name, value) in [
                ("content-type", "text/html; charset=utf-8"),
                ("cache-control", "no-store, no-transform"),
                ("retry-after", "3600"),
                ("x-robots-tag", "noindex"),
            ] {
                assert_eq!(
                    response
                        .headers()
                        .get(name)
                        .map(|header| header.to_str().unwrap()),
                    Some(value),
                    "{context}: header {name}"
                );
            }

            assert!(
                response.headers().get("x-powered-by").is_none(),
                "{context}: x-powered-by is set"
            );

            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read the body");

            // HEAD carries the headers but no body, by the HTTP spec.
            if method == "HEAD" {
                assert!(body.is_empty(), "{context}: body should be empty");

                continue;
            }

            assert_eq!(body.as_ref(), expected.as_bytes(), "{context}: body");
        }
    }
}

#[tokio::test]
async fn the_body_carries_the_hosts_message() {
    for (host, expected) in [
        ("one.example", "One is isolated."),
        ("three.example", "Everything else is isolated."),
    ] {
        let request = Request::builder()
            .method("GET")
            .uri("/anything?x=1")
            .header("host", host)
            .body(Body::empty())
            .expect("build the request");

        let response = router(Config::load(
            r#"{"one.example":"One is isolated."}"#,
            "Everything else is isolated.",
        ))
        .oneshot(request)
        .await
        .expect("the router is infallible");

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the body");
        let body = String::from_utf8(body.to_vec()).expect("the page is utf-8");

        assert!(body.contains(&format!("<p>{expected}</p>")), "host {host}");
    }
}

/// Cloud's startup probe arrives over IPv6, so the wildcard bind is the one
/// thing a unit test on the router cannot prove. This drives a real socket.
#[tokio::test]
async fn it_answers_on_the_ipv6_wildcard() {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0u16; 8], 0)))
        .await
        .expect("bind the IPv6 wildcard");
    let port = listener.local_addr().expect("local address").port();

    tokio::spawn(async move {
        let app = router(Config::load(r#"{"probe.example":"Probe message."}"#, ""));

        axum::serve(listener, app).await.ok();
    });

    let mut stream = tokio::net::TcpStream::connect(("::1", port))
        .await
        .expect("connect over IPv6");

    stream
        .write_all(
            b"GET /wp-admin/x.php?a=1 HTTP/1.1\r\nHost: probe.example\r\nConnection: close\r\n\r\n",
        )
        .await
        .expect("write the request");

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .expect("read the response");

    let response = String::from_utf8_lossy(&raw);

    assert!(
        response.starts_with("HTTP/1.1 503 Service Unavailable"),
        "status line: {:?}",
        response.lines().next()
    );
    assert!(
        response.contains("retry-after: 3600"),
        "retry-after is missing"
    );
    assert!(
        response.contains("Probe message."),
        "the host's message is missing"
    );
}
