# isolate

A stand-in app for Isolated Mode. When a Laravel Cloud app has to stop running PHP in a hurry, its
custom domain gets moved here, and every request to it (any method, any path, any query string)
gets one static HTML page back with a `503`.

The repository has two long-lived branches, one per runtime, and they are never merged into each
other:

| Branch | Runtime | Cloud environment | Code |
| --- | --- | --- | --- |
| `main` | Go (stdlib only) | `production` | `main.go`, `page.go` |
| `rust` | Rust (axum + tokio) | `rust` | `src/main.rs`, `src/page.rs` |

Two runtimes because the point is to get off the runtime that is being exploited. If the hole is in
Go, the domain goes to the Rust environment, and the other way round.

`templates/isolated.html` is the page, and it is byte-identical on both branches. Each branch's test
suite pins its SHA-256, so editing it on one side turns the other side red until both match.

## Configuration

| Variable | Meaning |
| --- | --- |
| `ISOLATE_MESSAGES` | JSON object, host → message, e.g. `{"example.com":"Back soon."}` |
| `ISOLATE_DEFAULT_MESSAGE` | Message for a host not listed above |
| `PORT` | Listen port, `3000` by default |

Both are read once at boot, so a change needs a redeploy. The message is HTML-escaped before it goes
into the page. A malformed `ISOLATE_MESSAGES` is logged and ignored rather than fatal: the server
comes up and serves the default.

## Branch work

Feature branches are `feat/<slug>` and target whichever of the two branches they belong to. CI on
`main` runs `go vet` and `go test`; CI on `rust` runs `cargo clippy -- -D warnings` and `cargo test`.
