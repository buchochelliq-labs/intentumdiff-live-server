# Changelog

Versions use the product version in Cargo SemVer form (`0.0.3-beta.1` is product `0.0.3b1`).

## 0.0.3-beta.1 (unreleased)

- Engine pinned to intentumdiff-core `74618c3` (core #252, core PR #312: review completeness).
  The reviewed parser wheel is still the one built at core `e60a55c`; it is re-pinned
  separately, and until then the CI `require_matching_native_core` gate fails closed.
- Review frames forward core #252's `completeness`, `incomplete` (including the sanitised
  `report`) and `metadata.equivalence_proof` verbatim: `diff.diff` and every
  `review.commit_diff.file_diffs[]` entry. Additive within protocol v2; the envelope is
  unchanged for complete and equivalent reviews. The server never opens `report.issue_url`
  or sends a report.
- `hello` negotiates `protocol_version` through Core (`unsupported_protocol_version`,
  `invalid_protocol_version`, `supported_protocol_versions`).
- Bounded byte framing: invalid UTF-8 frames get `invalid_utf8` and the session continues;
  stdin read errors (`io_error`, exit 1) are distinct from clean EOF (exit 0).
- Cancellation is advertised truthfully (`cancel_interrupts: false`, `interrupted: false`).
- The effective startup config is validated before `ready` (`invalid_config`, exit 2) and
  reported as `ready.config`.
- CI builds, tests and packages linux/darwin/win32 x64 and arm64 runtimes with checksums and
  provenance (`server_version`, `platform`, `rust_target`).

## 0.1.0

Pre-split crate version used through product 0.0.2 (native runtime: Linux x64 CI artifact).
