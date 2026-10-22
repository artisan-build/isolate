use std::net::SocketAddr;

use tokio::signal::unix::{signal, SignalKind};

use isolate::page::Config;
use isolate::server::router;

#[tokio::main]
async fn main() {
    let config = Config::load(
        &std::env::var("ISOLATE_MESSAGES").unwrap_or_default(),
        &std::env::var("ISOLATE_DEFAULT_MESSAGE").unwrap_or_default(),
    );

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

    axum::serve(listener, router(config))
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
