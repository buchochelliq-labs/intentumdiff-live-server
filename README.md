# intentdiff-live-server

[![CI](https://github.com/buchochelliq-labs/intentdiff-live-server/actions/workflows/ci.yml/badge.svg)](https://github.com/buchochelliq-labs/intentdiff-live-server/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust 1.93](https://img.shields.io/badge/rust-1.93-orange.svg)](https://www.rust-lang.org/)

The **native IntentDiff live-server** — the keystroke-level diff/review IPC server the
editor integrations spawn. An in-process consumer of the engine
([intentdiff-core](https://github.com/buchochelliq-labs/intentdiff-core)): it links the
core natively ("a binding that speaks sockets instead of function calls") and wraps it
with the transport (unix socket / Windows named pipe, accept loops, debounce, shutdown).

## Build

```bash
cargo build --release    # -> target/release/intentdiff-live-server
cargo test
```

Toolchain: Rust 1.93.0 (pinned in CI).

## Provenance

Migrated files-only (no history) from the IntentDiff monorepo
(`buchochelliq-labs/intentdiff`), which remains the archive of record. License: MIT.
