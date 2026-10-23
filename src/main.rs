use std::net::SocketAddr;

use tokio::signal::unix::{signal, SignalKind};

use isolate::content::ContentResolver;
use isolate::server::router;
use isolate::store::object_store_from_env;

#[tokio::main]
async fn main() {
    let resolver = ContentResolver::production(object_store_from_env(|name| std::env::var(name)));

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(3000);

    // The IPv6 wildcard accepts IPv4 connections too on Linux, and Cloud's
    // startup probe arrives over IPv6. Never narrow this to 0.0.0.0 or loopback.
    let addr = SocketAddr::from(([0u16; 8], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|error| panic!("isolate: cannot bind {addr}: {error}"));

    println!("isolate: listening on {addr}");

    axum::serve(listener, router(resolver))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap_or_else(|error| panic!("isolate: serve: {error}"));
}

/// Resolves on `SIGTERM` or `SIGINT` so axum can drain in-flight requests.
async fn shutdown_signal() {
    let mut sigterm = signal(SignalKind::terminate()).expect("install the SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("install the SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {},
        _ = sigint.recv() => {},
    }

    println!("isolate: received shutdown signal");
}
