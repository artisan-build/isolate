//! The router: one handler for every request that reaches the process.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Router;

use crate::page::{render_page, Config, PAGE_TEMPLATE};

/// Builds a router with no routes at all, so every method, path and query
/// string lands on the one fallback handler.
pub fn router(config: Config) -> Router {
    Router::new().fallback(isolate).with_state(Arc::new(config))
}

/// Answers with the isolation page for the requested host. It extracts only the
/// request's `Parts`, so the body is never read.
async fn isolate(State(config): State<Arc<Config>>, parts: Parts) -> Response {
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| parts.uri.authority().map(|authority| authority.as_str()))
        .unwrap_or_default();

    let body = render_page(PAGE_TEMPLATE, config.message_for(host));

    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::RETRY_AFTER, "3600")
        .header("x-robots-tag", "noindex")
        .body(Body::from(body))
        .expect("the isolation response is built from constants")
}
