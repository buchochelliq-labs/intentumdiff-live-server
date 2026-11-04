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

CI produces `native-linux-x64-<server commit>` containing a tarball, its SHA-256 and
provenance identifying the native server, engine and parser source. The tarball preserves
the executable permission and includes `wasm/` beside the binary, allowing zero-setup
parser discovery. Consumers must pin the CI artifact identity and check its checksum.

The native server currently uses stdio and terminal responses; cancellation acknowledges
requests but does not interrupt synchronous computation. Runtime parsers may also be
selected with `--wasm-dir` or `INTENTUMDIFF_WASM_DIR`.
