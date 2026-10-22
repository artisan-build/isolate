# Workflow — isolate

Project profile for the `multi-agent-build` skill. The coordinator agent reads this FIRST.

## Phase & mode
- phase: pre-launch (Isolated Mode spike, 2026-10-22)
- default mode: A-autonomous
- merge_policy: merge when CI green (Ed, 2026-10-22). Until the first PR lands CI with tests + static analysis, there is nothing to gate on: that PR merges after the worker's own local gate is green.
- merge method: `gh pr merge --squash --delete-branch`

## Hard gate
- command: main (Go): `go vet ./... && go test ./...`; branch rust: `cargo clippy -- -D warnings && cargo test`
- extra suites: none
- monorepo: no

## CI (the merge gate for Mode A)
- status: inadequate — none yet; the first PR adds it
- minimum bar: CI MUST include (1) testing and (2) static analysis.

## Dependency install (fresh worktree)
- command: main: `go mod download`; rust: `cargo fetch`
- post-install: none

## Agent-role constraints and exceptions
- constraints: none
- overrides: none
- invocation quirks: none

## Toolchain conformance
Standing ride-along rule: run the full gate when finalizing every PR; whatever it changes rides along as its own commit.

## Ship details
- branch naming: feat/<slug>
- PR target repo: artisan-build/isolate
- release / split steps: none

## Plan & coordination
- plan location: ~/Herd/brain/ideas/2026-10-22-isolated-mode.md
- Solo project: isolate
- run-log: ~/Herd/brain/projects/isolate/log.md

## Stack notes / quirks
- ONE Cloud app, TWO environments, each a different runtime detected at environment creation from its branch:
  - `main` has `go.mod` → the default `production` environment is the Go runtime (POST /api/applications ignores `branch`).
  - `rust` branch is a plain Cargo crate with NO go.mod → a second environment created via POST /api/applications/{app}/environments on that branch gets the Rust runtime.
  Source: brain projects/campfire-rust-experiment/writeup-addendum-native-runtime.md.
- Each binary serves one static page for EVERY path and query string, chosen by Host header. No PHP, no database, no cache, no dependencies beyond the standard library (Go) / axum+tokio (Rust).
- Listen on `PORT` (default 3000), IPv6-capable: Go `":"+port`; Rust bind `::`. Rust start command is the alphabetically first executable in /var/www/bin unless set — set `default-run` BEFORE creating the env.
- PRs on `rust` target `rust`, not main; the two branches never merge into each other.
