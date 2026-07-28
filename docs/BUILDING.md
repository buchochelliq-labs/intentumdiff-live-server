# Building intentdiff-live-server

Toolchain: **Rust 1.93.0**.

```bash
cargo build --release      # -> target/release/intentdiff-live-server
cargo test
```

The engine dependency is a git dep on
[intentdiff-core](https://github.com/buchochelliq-labs/intentdiff-core) pinned by tag; for a
private clone set `CARGO_NET_GIT_FETCH_WITH_CLI=true`. Parser components are resolved at
runtime from the wasm dir the spawning editor supplies (`$INTENTDIFF_WASM_DIR` or a directory
beside the binary).
