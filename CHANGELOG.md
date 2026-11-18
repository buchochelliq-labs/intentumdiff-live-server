# Changelog

Versions use the product version in Cargo SemVer form (`0.0.3-beta.1` is product `0.0.3b1`).

## 0.0.3-beta.1 (unreleased)

- Engine pinned to intentumdiff-core `1c4bdbc` (core #312 review completeness, #320 number
  respellings, #321 native Python operator ids, #323 Windows ARM64 PowerShell component).
  The bundled parser components come from the reviewed intentumdiff-python `0.0.3b1` wheel
  built at the same core `1c4bdbc` (python `99aeb3e`, CI run 44279129426, artifact
  14421310046, wheel sha256 `42f473ae…34ec`), which stages the registry `eb02686`
  (registry #20/#21) parser components. `require_matching_native_core` checks that the
  native binary and the wheel were built from the same core.
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
- **Not published in 0.0.3.** CI builds and tests the native runtime for every platform
  above, but 0.0.3 publishes no native binaries or release assets (#25). Build it from
  source (`cargo build --locked --release`) to use it; the VS Code extension's `auto`
  engine keeps using the Python runtime.

## 0.1.0

Pre-split crate version used through product 0.0.2 (native runtime: Linux x64 CI artifact).
