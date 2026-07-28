# Contributing to intentdiff-live-server

- This repo owns **transport only** (sockets/pipes, framing, lifecycle). Diff/review compute
  lives in the engine's `live_*_impl` handlers — protocol semantics change there, not here.
- Build + test per [docs/BUILDING.md](docs/BUILDING.md).
- Security posture matters: keep the Windows pipe DACL owner-only, bound request sizes, and
  never let a malformed request escape as anything but a structured error response.
