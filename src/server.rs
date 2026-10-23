//! The router: one page for every request except an exact logo request.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::request::Parts;
use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use axum::Router;

use crate::content::ContentResolver;
use crate::page::{render_page, PAGE_TEMPLATE};

pub const BUCKET_REQUEST_BUDGET: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct AppState {
    resolver: ContentResolver,
    budget: Duration,
}

pub fn router(resolver: ContentResolver) -> Router {
    router_with_budget(resolver, BUCKET_REQUEST_BUDGET)
}

pub fn router_with_budget(resolver: ContentResolver, budget: Duration) -> Router {
    Router::new()
        .fallback(isolate)
        .with_state(Arc::new(AppState { resolver, budget }))
}

async fn isolate(State(state): State<Arc<AppState>>, parts: Parts) -> Response {
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| parts.uri.authority().map(|authority| authority.as_str()))
        .unwrap_or_default();

    let content = tokio::time::timeout(state.budget, state.resolver.resolve(host))
        .await
        .unwrap_or_else(|_| state.resolver.last_good_or_built_in(host));
    if is_logo_request(&parts) {
        if let Some(logo) = content.logo {
            let body = if parts.method == Method::HEAD {
                Body::empty()
            } else {
                Body::from(logo.bytes)
            };
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, logo.media_type)
                .header(header::CACHE_CONTROL, "no-store, no-transform")
                .header("x-content-type-options", "nosniff")
                .body(body)
                .expect("logo response uses valid constants");
        }
    }

    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store, no-transform")
        .header(header::RETRY_AFTER, "3600")
        .header("x-robots-tag", "noindex")
        .body(Body::from(render_page(PAGE_TEMPLATE, &content)))
        .expect("page response uses valid constants")
}

fn is_logo_request(parts: &Parts) -> bool {
    matches!(parts.method, Method::GET | Method::HEAD)
        && parts.uri.path() == "/__isolate/logo"
        && parts.uri.query().is_none()
}
