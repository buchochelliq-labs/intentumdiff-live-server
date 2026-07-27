# intentdiff-live-server

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
