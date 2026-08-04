# intentumdiff-live-server

[![CI](https://github.com/buchochelliq-labs/intentumdiff-live-server/actions/workflows/ci.yml/badge.svg)](https://github.com/buchochelliq-labs/intentumdiff-live-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust 1.95](https://img.shields.io/badge/rust-1.95-orange.svg)](https://www.rust-lang.org/)

The **native IntentumDiff live-server** — the keystroke-level diff/review IPC server the
editor integrations spawn. An in-process consumer of the engine
([intentumdiff-core](https://github.com/buchochelliq-labs/intentumdiff-core)): it links the
core natively ("a binding that speaks sockets instead of function calls") and wraps it
with the transport (unix socket / Windows named pipe, accept loops, debounce, shutdown).

## Build

```bash
cargo build --release    # -> target/release/intentumdiff-live-server
cargo test
```

Toolchain: Rust 1.93.0 (pinned in CI).

## Provenance

Migrated files-only (no history) from the IntentumDiff monorepo
(`buchochelliq-labs/intentumdiff`), which remains the archive of record. License: MIT.
