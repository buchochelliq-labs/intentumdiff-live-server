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
- `hello` negotiation is Core's `live_negotiate_protocol`: an absent/null `protocol_version`
  takes the current version (2); a supported integer is accepted; any other integer fails with
  `unsupported_protocol_version`, any non-integer with `invalid_protocol_version`. Responses
  (success and error) carry `supported_protocol_versions`, as does the `ready` line.
- Framing is byte-level and bounded (`max_line_bytes`) before UTF-8 decoding. An oversized
  frame gets `line_too_large`, a frame that is not UTF-8 gets `invalid_utf8` (both with
  `seq` 0, `op` "error"); the session continues at the next `\n`. Clean stdin EOF exits 0;
  a genuine stdin read failure emits `io_error` and exits 1.
- Requests are served synchronously in arrival order. `cancel` is accepted but cannot reach
  in-flight work: capabilities advertise `cancel_interrupts: false` and cancel responses say
  `cancelled: 0, interrupted: false`. Real interruption needs an engine-owned cancellation/
  deadline hook through guest and native phases plus a reader decoupled from compute.
- Debounced re-diff on keystroke updates; bounded request sizes; explicit shutdown op so the
  editor can terminate the server cleanly (no orphan processes).

- Startup validates the effective configuration (project `intentumdiff.yaml` or
  `--config-json`, plus `--fuel`) before `ready`: unknown keys, wrong types, out-of-range and
  unsupported values print `ready` with `ok:false`, `invalid_config`, and exit 2. A successful
  `ready` reports the effective `config`.

## Review completeness (core #252)

Every served review carries core's `completeness` (`complete` | `equivalent` | `incomplete`),
`incomplete` (`null`, or core's object with `reason`, `message`, `report_url`, `language`,
`core_version`, `source_ranges` and the sanitised `report`) and `metadata.equivalence_proof`.
The `diff` response's `diff` and each `review` response's `commit_diff.file_diffs[]` carry
them exactly as core produced them; this transport neither adds, renames nor recomputes any
of them, and the response envelope (`op`, `seq`, `ok`, `metadata` summary) is the same for all
three states. The fields are additive within **protocol version 2** (no version bump: a v2
client that ignores them still parses every frame). An `incomplete` review has
`changes: []` and `metadata.change_count: 0` but is NOT clean: clients must render
`incomplete.message`. The server never opens `report.issue_url` or sends a report; per
[C_ABI.md](https://github.com/buchochelliq-labs/intentumdiff-core/blob/main/docs/C_ABI.md#review-completeness-core-252)
the client must preview `report.title`/`report.body_markdown` and get explicit user
confirmation before opening it.

## Ops

The protocol mirrors the engine's live surface: capabilities/limits handshake, single-file
live diff (buffer content vs a git ref), working-tree/commit review, and config loading —
request parsing and responses are engine handlers, so every consumer (this server, the CLI,
the language bindings) serves identical results.

The VS Code extension bundles this binary and spawns it
([intentumdiff-vscode](https://github.com/buchochelliq-labs/intentumdiff-vscode)); any other
editor can do the same.
