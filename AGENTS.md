# Agent instructions — intentdiff-live-server

The native editor IPC server. Transport ONLY — diff/review compute is engine handlers.

## Hard invariants
- Protocol semantics change in intentdiff-core's live_*_impl, never here.
- Windows named pipe keeps its owner-only DACL; requests stay bounded; malformed input gets
  a structured error response, never a crash.

Build: `cargo build --release && cargo test` (Rust 1.93.0; core git dep needs
CARGO_NET_GIT_FETCH_WITH_CLI=true for private clones).
Map: docs/ARCHITECTURE.md (transport + protocol) · docs/BUILDING.md.
