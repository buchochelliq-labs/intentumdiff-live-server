# Building intentumdiff-live-server

Toolchain: **Rust 1.95.0** (the CI version).

```bash
# Requires a token that can read the reviewed private parser artifact.
GH_TOKEN=... python scripts/provision_components.py
export INTENTUMDIFF_TEST_WASM_DIR="$PWD/dist/wasm"
export CARGO_NET_GIT_FETCH_WITH_CLI=true
cargo build --locked --release
cargo test --locked
cargo test --locked --release
python scripts/package_native.py
```

For offline provisioning, pass `--wheel /path/to/reviewed.whl`. The wheel must match the
immutable SHA-256 in `scripts/provision_components.py`; arbitrary wheels are refused.
Tests fail if their parser manifest or Git is missing. They exercise the real Rust process,
including diff, review, image artifacts, self-ignoring cache, path rejection and EOF shutdown.

The engine dependency is pinned to a complete commit in `Cargo.toml` and `Cargo.lock`.
CI downloads one exact reviewed parser artifact, checks its archive and wheel checksums,
then verifies the complete component provenance manifest before extracting Wasm files.
The wheel supplies build inputs only: the shipped runtime contains no Python package or
interpreter. If the upstream CI artifact expires, provisioning fails closed; update its
immutable identity only after verifying replacement components.

CI builds and tests one native runtime per supported VS Code target — `linux-x64`,
`linux-arm64`, `darwin-x64`, `darwin-arm64`, `win32-x64`, `win32-arm64` — on a native runner
for each, and uploads `native-<platform>-<server commit>`: an archive
(`intentumdiff-live-server-<platform>.tar.gz`, `.zip` on Windows), its `SHA256SUMS`, and
`provenance.json` identifying the server commit and version, platform, Rust target, engine
and parser source. The archive preserves the executable permission, includes `wasm/` beside
the binary (zero-setup parser discovery) and an inner `SHA256SUMS`. Consumers must pin the
CI artifact identity and check its checksum (`python scripts/package_native.py --verify DIR`).
These are CI artifacts only; nothing is published to a release from this workflow.

Versioning: `Cargo.toml` carries the product version in SemVer prerelease form
(`0.0.3-beta.1` is product `0.0.3b1`). It is reported as `server_version` in the `ready`
line and in `provenance.json`.

The native server currently uses stdio and terminal responses. Requests run to completion in
order, so cancellation cannot interrupt work: `capabilities.cancel_interrupts` is `false` and
every `cancel` is answered `{"ok":true,"cancelled":0,"interrupted":false}`. Runtime parsers may also be
selected with `--wasm-dir` or `INTENTUMDIFF_WASM_DIR`.
