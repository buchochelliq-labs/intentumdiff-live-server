//! Native IntentDiff live-server (#100, A2.5 Phase B — first cut): the stdio transport.
//!
//! Links the PURE-RUST engine (`intentdiff-rust-core` with `default-features = false` — no
//! pyo3, no libpython anywhere in the process) and speaks the same JSON-line protocol v2 as
//! the Python `intentdiff live-server`: a `ready` line on startup, then one response (or
//! error) per request line for `hello` / `diff` / `review` / `cancel`, with the same
//! validation/limits/capabilities (all shared with the Python server via the engine's
//! `live_server` protocol impls — the wire contract is defined ONCE).
//!
//! First-cut scope (deliberate):
//! - **stdio only.** The unix-socket + Windows named-pipe transports carry the #88
//!   security controls (0o700 socket dir / Protected DACL) and land as their own careful
//!   slice — not rushed into this one.
//! - **No server-side debounce.** The VS Code extension already debounces client-side;
//!   per-path debounce timers arrive with the socket transport.
//! - **No streaming.** Stream requests are served as a single terminal response.
//! - **`{"fallback": …}` becomes an error response.** In the Python shell a fallback
//!   routes to the Python differ; this binary has no Python to fall back to, so the
//!   uncovered cases (unresolved extension, in-effect guardrail policy, generic/markdown,
//!   finalize declined) surface honestly as `native_fallback` errors.
//! - Clean shutdown on stdin EOF (#73: no threads, no handles to orphan).

use std::io::{self, BufRead, Write};

use intentdiff_rust_core::live_server as proto;
use serde_json::{json, Value};

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The repo path, CLI-compatible with the Python `intentdiff live-server` invocation the VS Code
/// extension builds (`buildLiveServerArgs`): `<exe> live-server <workspaceRoot> --stdio --ref R
/// --debounce S [--fuel N]`. The binary is a drop-in — pointing the `intentdiff.executable`
/// setting at it is the whole cutover. A leading `live-server` token is skipped; the first
/// non-flag positional is the repo; `--repo <path>` also works (this binary's native flag).
fn repo_from_args(args: &[String]) -> Option<String> {
    if let Some(repo) = arg_value(args, "--repo") {
        return Some(repo);
    }
    let value_flags = [
        "--repo",
        "--ref",
        "--debounce",
        "--fuel",
        "--wasm-dir",
        "--config-json",
    ];
    let mut skip_next = false;
    for (i, a) in args.iter().enumerate().skip(1) {
        if skip_next {
            skip_next = false;
            continue;
        }
        if value_flags.contains(&a.as_str()) {
            skip_next = true;
            continue;
        }
        if a.starts_with("--") {
            continue; // boolean flags like --stdio
        }
        if i == 1 && a == "live-server" {
            continue; // the CLI subcommand token
        }
        return Some(a.clone());
    }
    None
}

fn dir_has_manifest(dir: &std::path::Path) -> bool {
    dir.join("parser_manifest.json").is_file()
}

/// Bundled-parser dir, ZERO-SETUP by design (a GUI-launched VS Code passes no env vars — the
/// first side-by-side failed on exactly that): `--wasm-dir` flag > `INTENTDIFF_WASM_DIR` env >
/// `<exe dir>/wasm` (the split-repo shipping shape) > DEV-LAYOUT auto-detection (walk the exe's
/// ancestors for `src/intentdiff/wasm` — finds the monorepo wasm when the exe runs from
/// `crates/live-server/target/<profile>/`). Every candidate is verified by the presence of
/// `parser_manifest.json`, never guessed from a bare directory.
fn resolve_wasm_dir(args: &[String]) -> String {
    if let Some(dir) = arg_value(args, "--wasm-dir") {
        return dir;
    }
    if let Ok(dir) = std::env::var("INTENTDIFF_WASM_DIR") {
        if !dir.trim().is_empty() {
            return dir;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let shipped = exe_dir.join("wasm");
            if dir_has_manifest(&shipped) {
                return shipped.to_string_lossy().into_owned();
            }
            for ancestor in exe_dir.ancestors() {
                let dev = ancestor.join("src").join("intentdiff").join("wasm");
                if dir_has_manifest(&dev) {
                    return dev.to_string_lossy().into_owned();
                }
            }
        }
    }
    String::new()
}

fn parse_json(s: &str) -> Option<Value> {
    serde_json::from_str(s).ok()
}

fn write_line(out: &mut impl Write, value: &Value) {
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn error_response(seq: i64, code: &str, message: &str, op: &str) -> Value {
    parse_json(&proto::live_error_response_impl(seq, code, message, op))
        .unwrap_or_else(|| json!({ "op": op, "seq": seq, "ok": false }))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let repo = repo_from_args(&args).unwrap_or_else(|| ".".to_owned());
    let default_ref = arg_value(&args, "--ref").unwrap_or_else(|| "HEAD".to_owned());
    let wasm_dir = resolve_wasm_dir(&args);
    // Engine config, matching the Python server's `_load_cli_config(config_start_path=repo)`
    // exactly: an explicit --config-json wins; otherwise the repo's intentdiff.yaml `config:`
    // section loads natively (#99 loader), with the extension CLI's --fuel overriding plugin_fuel
    // on top. A malformed config file degrades to defaults WITH a ready-line warning (the Python
    // CLI exits instead; a keystroke server staying alive with a loud warning is kinder).
    // (--stdio is a no-op — this transport IS stdio; --debounce is accepted and ignored, the
    // extension debounces client-side.)
    let mut config_warnings: Vec<String> = Vec::new();
    let (config_json, config_source) = match arg_value(&args, "--config-json") {
        Some(explicit) => (explicit, "flag"),
        None => match proto::live_load_project_config_impl(&repo) {
            Ok(loaded) if loaded.trim() != "{}" => (loaded, "intentdiff.yaml"),
            Ok(_) => ("{}".to_owned(), "defaults"),
            Err(e) => {
                config_warnings.push(format!(
                    "intentdiff.yaml config not loaded ({e}); using engine defaults"
                ));
                ("{}".to_owned(), "defaults")
            }
        },
    };
    let config_json = match arg_value(&args, "--fuel").and_then(|f| f.parse::<i64>().ok()) {
        Some(fuel) => {
            let mut value: Value = serde_json::from_str(&config_json).unwrap_or_else(|_| json!({}));
            value["plugin_fuel"] = json!(fuel);
            value.to_string()
        }
        None => config_json,
    };

    let stdout = io::stdout();
    let mut out = stdout.lock();

    let limits = parse_json(&proto::live_limits_impl()).unwrap_or_else(|| json!({}));
    let capabilities = parse_json(&proto::live_capabilities_impl()).unwrap_or_else(|| json!({}));
    let mut ready_warnings = config_warnings.clone();
    if wasm_dir.is_empty() {
        ready_warnings.push(
            "bundled parsers NOT FOUND: pass --wasm-dir or set INTENTDIFF_WASM_DIR \
             (exe-adjacent wasm/ and dev-layout auto-detection both failed); every \
             diff/review will fail with wasm_dir_missing"
                .to_owned(),
        );
    }
    // wasm_dir in the ready line makes a missing-parser setup visible in ANY trace instead of
    // surfacing as misleading per-file "no bundled parser" errors (the first side-by-side bug).
    write_line(
        &mut out,
        &json!({
            "op": "ready",
            "ok": true,
            "protocol_version": 2,
            "transport": "stdio",
            "repo_path": repo,
            "ref": default_ref,
            "limits": limits,
            "capabilities": capabilities,
            "wasm_dir": if wasm_dir.is_empty() { Value::Null } else { json!(wasm_dir) },
            "config_source": config_source,
            "warnings": ready_warnings,
        }),
    );

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // stdin closed/errored -> clean shutdown
        };
        if line.trim().is_empty() {
            continue;
        }
        if line.len() as u64 > proto::MAX_LINE_BYTES {
            write_line(
                &mut out,
                &error_response(0, "line_too_large", "request line exceeds limit", "error"),
            );
            continue;
        }

        // seq (shared validation: number/numeric-string/bool coerce; null/list/object reject).
        let seq = match proto::live_request_seq_impl(&line) {
            Ok(seq_json) => match parse_json(&seq_json) {
                Some(v) if v.get("error").is_some() => {
                    write_line(&mut out, &v["error"]);
                    continue;
                }
                Some(v) => v.get("seq").and_then(Value::as_i64).unwrap_or(0),
                None => 0,
            },
            Err(_) => {
                write_line(
                    &mut out,
                    &error_response(0, "invalid_json", "request is not valid JSON", "error"),
                );
                continue;
            }
        };

        let request = match parse_json(&line) {
            Some(v) => v,
            None => {
                write_line(
                    &mut out,
                    &error_response(seq, "invalid_json", "request is not valid JSON", "error"),
                );
                continue;
            }
        };
        // Absent op = the legacy diff request shape (path/content only).
        let op = request.get("op").and_then(Value::as_str).unwrap_or("diff");

        // Without the bundled parsers no diff/review can serve — say so EXPLICITLY per request
        // rather than letting every file surface a misleading "no bundled parser" fallback.
        if wasm_dir.is_empty() && (op == "diff" || op == "review") {
            write_line(
                &mut out,
                &error_response(
                    seq,
                    "wasm_dir_missing",
                    "bundled parsers not found: pass --wasm-dir or set INTENTDIFF_WASM_DIR \
                     (exe-adjacent wasm/ and dev-layout auto-detection failed)",
                    op,
                ),
            );
            continue;
        }

        match op {
            "hello" => write_line(
                &mut out,
                &json!({
                    "op": "hello",
                    "seq": seq,
                    "ok": true,
                    "protocol_version": 2,
                    "repo_path": repo,
                    "ref": default_ref,
                    "limits": parse_json(&proto::live_limits_impl()).unwrap_or_else(|| json!({})),
                    "capabilities": parse_json(&proto::live_capabilities_impl())
                        .unwrap_or_else(|| json!({})),
                }),
            ),
            "cancel" => write_line(
                &mut out,
                &json!({ "op": "cancel", "seq": seq, "ok": true, "cancelled": 0 }),
            ),
            "diff" => {
                let parsed = match proto::live_parse_diff_request_impl(&line, &default_ref) {
                    Ok(p) => p,
                    Err(e) => {
                        write_line(&mut out, &error_response(seq, "invalid_request", &e, "diff"));
                        continue;
                    }
                };
                let parsed: Value = match parse_json(&parsed) {
                    Some(v) if v.get("error").is_some() => {
                        write_line(&mut out, &v["error"]);
                        continue;
                    }
                    Some(v) => v["parsed"].clone(),
                    None => {
                        write_line(
                            &mut out,
                            &error_response(seq, "invalid_request", "unparseable request", "diff"),
                        );
                        continue;
                    }
                };
                let path = parsed.get("path").and_then(Value::as_str).unwrap_or("");
                let content = parsed.get("content").and_then(Value::as_str).unwrap_or("");
                let git_ref = parsed
                    .get("ref")
                    .and_then(Value::as_str)
                    .unwrap_or(&default_ref);
                let start = std::time::Instant::now();
                match proto::live_handle_diff_impl(
                    &repo, path, git_ref, content, &config_json, &wasm_dir,
                ) {
                    Ok(result) => {
                        let result: Value = parse_json(&result).unwrap_or_else(|| json!({}));
                        if let Some(diff) = result.get("diff") {
                            let duration_ms =
                                (start.elapsed().as_secs_f64() * 1000.0 * 1000.0).round() / 1000.0;
                            let change_count = diff
                                .get("changes")
                                .and_then(Value::as_array)
                                .map(|a| a.len())
                                .unwrap_or(0);
                            write_line(
                                &mut out,
                                &json!({
                                    "op": "diff",
                                    "seq": seq,
                                    "ok": true,
                                    "diff": diff,
                                    "metadata": {
                                        "duration_ms": duration_ms,
                                        "language": diff.get("language").cloned()
                                            .unwrap_or(Value::Null),
                                        "change_count": change_count,
                                        "style_only": diff.get("is_style_only").cloned()
                                            .unwrap_or(Value::Bool(false)),
                                        "guardrail_violation_count": diff
                                            .get("guardrail_violations")
                                            .and_then(Value::as_array)
                                            .map(|a| a.len())
                                            .unwrap_or(0),
                                    },
                                }),
                            );
                        } else {
                            let reason = result
                                .get("fallback")
                                .and_then(Value::as_str)
                                .unwrap_or("engine returned no diff");
                            write_line(
                                &mut out,
                                &error_response(seq, "native_fallback", reason, "diff"),
                            );
                        }
                    }
                    Err(e) => {
                        write_line(&mut out, &error_response(seq, "diff_error", &e, "diff"))
                    }
                }
            }
            "review" => {
                let parsed = match proto::live_parse_review_request_impl(&line, &default_ref, seq) {
                    Ok(p) => p,
                    Err(e) => {
                        write_line(
                            &mut out,
                            &error_response(seq, "invalid_request", &e, "review"),
                        );
                        continue;
                    }
                };
                let parsed: Value = match parse_json(&parsed) {
                    Some(v) if v.get("error").is_some() => {
                        write_line(&mut out, &v["error"]);
                        continue;
                    }
                    Some(v) => v["parsed"].clone(),
                    None => {
                        write_line(
                            &mut out,
                            &error_response(seq, "invalid_request", "unparseable request", "review"),
                        );
                        continue;
                    }
                };
                let old_ref = parsed
                    .get("old_ref")
                    .and_then(Value::as_str)
                    .unwrap_or(&default_ref);
                let new_ref = parsed.get("new_ref").and_then(Value::as_str).unwrap_or("");
                match proto::live_handle_review_impl(
                    &repo, old_ref, new_ref, &config_json, &wasm_dir,
                ) {
                    Ok(result) => {
                        let result: Value = parse_json(&result).unwrap_or_else(|| json!({}));
                        if let Some(commit_diff) = result.get("commit_diff") {
                            let file_count = commit_diff
                                .get("file_diffs")
                                .and_then(Value::as_array)
                                .map(|a| a.len())
                                .unwrap_or(0);
                            let cross_count = commit_diff
                                .get("cross_file_changes")
                                .and_then(Value::as_array)
                                .map(|a| a.len())
                                .unwrap_or(0);
                            write_line(
                                &mut out,
                                &json!({
                                    "op": "review",
                                    "seq": seq,
                                    "ok": true,
                                    "commit_diff": commit_diff,
                                    "metadata": {
                                        "file_count": file_count,
                                        "guardrail_violation_count": 0,
                                        "cross_file_change_count": cross_count,
                                        "streamed": false,
                                    },
                                }),
                            );
                        } else {
                            let reason = result
                                .get("fallback")
                                .and_then(Value::as_str)
                                .unwrap_or("engine returned no commit diff");
                            write_line(
                                &mut out,
                                &error_response(seq, "native_fallback", reason, "review"),
                            );
                        }
                    }
                    Err(e) => {
                        write_line(&mut out, &error_response(seq, "review_error", &e, "review"))
                    }
                }
            }
            other => write_line(
                &mut out,
                &error_response(seq, "invalid_op", &format!("unknown op: {other}"), "error"),
            ),
        }
    }
}
