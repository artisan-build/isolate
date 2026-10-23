# isolate

A stand-in app for Isolated Mode. When a Laravel Cloud app has to stop running PHP in a hurry, its
custom domain gets moved here. Requests receive the isolation page with a `503`; an exact `GET` or
`HEAD` to `/__isolate/logo` serves that site's configured logo when one is available.

The repository has two long-lived branches, one per runtime, and they are never merged into each
other:

| Branch | Runtime | Cloud environment | Code |
| --- | --- | --- | --- |
| `main` | Go | `production` | `main.go`, `content.go`, `page.go`, `store.go` |
| `rust` | Rust | `rust` | `src/main.rs`, `src/content.rs`, `src/page.rs`, `src/store.rs` |

Two runtimes because the point is to get off the runtime that is being exploited. If the hole is in
Go, the domain goes to the Rust environment, and the other way round.

`templates/isolated.html` is the shared page source. Each branch keeps complete built-in and customized
response fixtures under `testdata/`; those fixtures must remain byte-identical between runtimes.

## Configuration

Content comes from the private object-storage bucket attached to both Cloud environments. Put
site-specific files under `sites/<host>/` and shared fallbacks under `default/`. Messages and branding
refresh within ten seconds, without a deploy. See [`docs/content-contract.md`](docs/content-contract.md)
for the file layout, validation rules, and fallback order.

Laravel Cloud injects the bucket's `AWS_BUCKET`, credentials, region, endpoint, and path-style setting.
The binary accepts either `AWS_ENDPOINT_URL` or `AWS_ENDPOINT`, and either `AWS_REGION` or
`AWS_DEFAULT_REGION`. During a bucket outage it uses the last validated content it read, or the
built-in page if there is no earlier value. `PORT` controls the listen port and defaults to `3000`.

## Branch work

Feature branches are `feat/<slug>` and target whichever of the two branches they belong to. CI on
`main` runs `go vet` and `go test`; CI on `rust` runs `cargo clippy -- -D warnings` and `cargo test`.
