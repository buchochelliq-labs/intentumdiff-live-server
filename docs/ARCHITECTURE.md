# intentumdiff-live-server architecture

The native keystroke-level diff/review IPC server editors spawn. Modeled as an **in-process
consumer of the engine** — "a binding that speaks sockets instead of function calls": it links
[intentumdiff-core](https://github.com/buchochelliq-labs/intentumdiff-core) as a Rust rlib
(`default-features = false`) and calls the same `live_*_impl` handlers the
[C ABI](https://github.com/buchochelliq-labs/intentumdiff-core/blob/main/docs/C_ABI.md) exposes.
No IPC round-trip ever re-implements semantics.

## Transport

- **Unix domain socket** on Linux/macOS; **named pipe** on Windows, created with a restrictive
  DACL (owner-only) so other local users cannot connect.
- Line-delimited JSON requests/responses with sequence numbers; unsupported protocol versions
  and malformed requests return structured error responses (never a crash).
- Debounced re-diff on keystroke updates; bounded request sizes; explicit shutdown op so the
  editor can terminate the server cleanly (no orphan processes).

## Ops

The protocol mirrors the engine's live surface: capabilities/limits handshake, single-file
live diff (buffer content vs a git ref), working-tree/commit review, and config loading —
request parsing and responses are engine handlers, so every consumer (this server, the CLI,
the language bindings) serves identical results.

The VS Code extension bundles this binary and spawns it
([intentumdiff-vscode](https://github.com/buchochelliq-labs/intentumdiff-vscode)); any other
editor can do the same.
