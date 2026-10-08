//! End-to-end regression test for the native live-server (#100): spawn the built binary and
//! drive a full JSON-line protocol-v2 session over stdio, asserting the wire contract the VS
//! Code extension depends on — `ready`, `hello`, `diff`, `review`, `cancel`, clean EOF exit.
//! Self-contained (shells out to `git`, no Python); fails closed when verified
//! bundled Wasm parsers or `git` are unavailable.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

/// The dev-layout wasm dir (walk ancestors for `src/intentumdiff/wasm`), manifest-verified.
fn find_wasm_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("INTENTUMDIFF_TEST_WASM_DIR") {
        let path = PathBuf::from(dir);
        assert!(
            path.join("parser_manifest.json").is_file(),
            "test parser manifest missing"
        );
        return Some(path);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        let dev = ancestor.join("src").join("intentumdiff").join("wasm");
        if dev.join("parser_manifest.json").is_file() {
            return Some(dev);
        }
    }
    None
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git spawn");
    assert!(status.success(), "git {args:?} failed");
}

fn unique_temp_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "intentumdiff-live-it-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

struct Session {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    rx: mpsc::Receiver<Value>,
}

impl Session {
    fn send(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").expect("write");
        self.stdin.flush().expect("flush");
    }

    /// Blocking read of the next protocol line (debug core JITs wasm on first diff).
    fn recv(&self) -> Value {
        self.rx
            .recv_timeout(Duration::from_secs(120))
            .expect("timed out waiting for a protocol line")
    }
}

fn spawn(bin: &str, repo: &Path, wasm_dir: &Path) -> Session {
    spawn_with_extra_args(bin, repo, wasm_dir, &[])
}

fn spawn_with_extra_args(bin: &str, repo: &Path, wasm_dir: &Path, extra_args: &[&str]) -> Session {
    // Exercise the extension-shaped argv (`live-server <root> --stdio --ref R ...`).
    let mut child = Command::new(bin)
        .args([
            "live-server",
            &repo.to_string_lossy(),
            "--stdio",
            "--ref",
            "HEAD",
            "--wasm-dir",
            &wasm_dir.to_string_lossy(),
        ])
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn live-server");

    let stdout = child.stdout.take().expect("piped stdout");
    let stdin = child.stdin.take().expect("piped stdin");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { return };
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<Value>(&line) {
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }
    });

    Session { child, stdin, rx }
}

#[test]
fn native_live_server_serves_the_protocol() {
    let wasm_dir =
        find_wasm_dir().expect("set INTENTUMDIFF_TEST_WASM_DIR to verified parser components");
    assert!(git_available(), "git is required for protocol verification");
    let bin = env!("CARGO_BIN_EXE_intentumdiff-live-server");

    let base = unique_temp_dir();
    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "T"]);
    std::fs::write(repo.join("a.ts"), "const x: number = 1;\n").unwrap();
    git(&repo, &["add", "a.ts"]);
    git(&repo, &["commit", "-m", "v1"]);
    // An uncommitted working-tree change so the HEAD-vs-working-tree review has a file.
    std::fs::write(repo.join("a.ts"), "const x: number = 2;\n").unwrap();

    let mut s = spawn(bin, &repo, &wasm_dir);

    // 1. The ready line: protocol v2, stdio, the resolved repo + wasm dir.
    let ready = s.recv();
    assert_eq!(ready["op"], "ready");
    assert_eq!(ready["ok"], true);
    assert_eq!(ready["protocol_version"], 2);
    assert_eq!(ready["transport"], "stdio");
    assert_eq!(ready["ref"], "HEAD");
    assert!(
        ready["wasm_dir"].is_string(),
        "wasm_dir should be resolved: {ready}"
    );
    assert!(
        ready["capabilities"]["review"].as_bool().unwrap_or(false),
        "review capability should be advertised: {ready}"
    );

    assert_eq!(ready["capabilities"]["stream"], false);
    assert_eq!(ready["capabilities"]["review_streaming"], false);
    assert_eq!(ready["capabilities"]["edit_deltas"], false);

    // 2. hello echoes the protocol handshake.
    s.send(&json!({"op": "hello", "seq": 1}));
    let hello = s.recv();
    assert_eq!(hello["op"], "hello");
    assert_eq!(hello["seq"], 1);
    assert_eq!(hello["ok"], true);
    assert_eq!(hello["protocol_version"], 2);

    // 3. A live diff of the buffer against HEAD.
    s.send(&json!({
        "op": "diff", "seq": 2, "path": "a.ts",
        "content": "const x: number = 42;\n", "ref": "HEAD",
    }));
    let diff = s.recv();
    assert_eq!(diff["op"], "diff", "unexpected: {diff}");
    assert_eq!(diff["seq"], 2);
    assert_eq!(diff["ok"], true);
    assert_eq!(diff["diff"]["language"], "typescript");
    let change_types: Vec<&str> = diff["diff"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["change_type"].as_str())
        .collect();
    assert!(
        change_types.contains(&"MODIFICATION"),
        "got {change_types:?}"
    );

    // 4. A working-tree review (old_ref HEAD, no new_ref = the extension's default).
    s.send(&json!({"op": "review", "seq": 3, "old_ref": "HEAD"}));
    let review = s.recv();
    assert_eq!(review["op"], "review", "unexpected: {review}");
    assert_eq!(review["seq"], 3);
    assert_eq!(review["ok"], true);
    let file_diffs = review["commit_diff"]["file_diffs"].as_array().unwrap();
    assert!(
        file_diffs.iter().any(|f| {
            f["old_filename"].as_str() == Some("a.ts") || f["new_filename"].as_str() == Some("a.ts")
        }),
        "the review should include the modified a.ts: {review}"
    );

    // 5. cancel is answered (no in-flight work to cancel).
    s.send(&json!({"op": "cancel", "seq": 4}));
    let cancel = s.recv();
    assert_eq!(cancel["op"], "cancel");
    assert_eq!(cancel["ok"], true);

    std::fs::write(repo.join("card.png"), include_bytes!("fixtures/before.png")).unwrap();
    git(&repo, &["add", "card.png"]);
    git(&repo, &["commit", "-m", "image base"]);
    std::fs::write(repo.join("card.png"), include_bytes!("fixtures/after.png")).unwrap();
    s.send(&json!({"op": "asset_diff", "seq": 5, "path": "card.png"}));
    let asset = s.recv();
    assert_eq!(
        asset["op"], "asset_diff",
        "asset operation must be dispatched: {asset}"
    );
    assert_eq!(asset["ok"], true, "{asset}");
    assert_eq!(asset["result"]["status"], "compared");
    for layer in [
        "before",
        "after",
        "diff",
        "heatmap",
        "mask",
        "overlay",
        "contact_sheet",
    ] {
        let path = Path::new(
            asset["result"]["artifacts"][layer]
                .as_str()
                .expect("artifact path"),
        );
        assert!(path.is_file(), "missing artifact {layer}");
        assert!(
            path.starts_with(repo.join(".intentumdiff-cache")),
            "artifact escaped cache"
        );
    }
    assert!(
        std::fs::read_to_string(repo.join(".intentumdiff-cache/.gitignore"))
            .unwrap()
            .contains('*')
    );
    s.send(&json!({"op": "asset_diff", "seq": 6, "path": "../outside.png"}));
    let asset = s.recv();
    assert_eq!(
        asset["op"], "asset_diff",
        "asset operation must be dispatched: {asset}"
    );
    assert_eq!(asset["ok"], false);
    assert_eq!(asset["error"]["code"], "invalid_request");

    // 6. Clean shutdown on stdin EOF (#73).
    drop(s.stdin);
    let status = s.child.wait().expect("wait");
    assert!(status.success(), "clean exit expected, got {status:?}");

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn native_review_preserves_mixed_code_and_image_changes() {
    let wasm_dir =
        find_wasm_dir().expect("set INTENTUMDIFF_TEST_WASM_DIR to verified parser components");
    assert!(git_available(), "git is required for protocol verification");
    let base = unique_temp_dir();
    git(&base, &["init"]);
    git(&base, &["config", "user.email", "t@example.com"]);
    git(&base, &["config", "user.name", "T"]);
    std::fs::write(base.join("a.ts"), "const x: number = 1;\n").unwrap();
    std::fs::write(base.join("card.png"), include_bytes!("fixtures/before.png")).unwrap();
    git(&base, &["add", "."]);
    git(&base, &["commit", "-m", "mixed base"]);
    std::fs::write(base.join("a.ts"), "const x: number = 2;\n").unwrap();
    std::fs::write(base.join("card.png"), include_bytes!("fixtures/after.png")).unwrap();
    let mut session = spawn(
        env!("CARGO_BIN_EXE_intentumdiff-live-server"),
        &base,
        &wasm_dir,
    );
    assert_eq!(session.recv()["op"], "ready");
    session.send(&json!({"op": "review", "seq": 1, "old_ref": "HEAD"}));
    let response = session.recv();
    assert_eq!(
        response["ok"], true,
        "mixed review must preserve code and assets: {response}"
    );
    let files = response["commit_diff"]["file_diffs"]
        .as_array()
        .expect("file diffs");
    assert!(
        files.iter().any(|file| file["new_filename"] == "a.ts"),
        "missing code: {response}"
    );
    // Both Python and native text review skip binary bytes. VS Code requests the
    // image separately from its Git snapshot; exercise that same wire contract.
    session.send(&json!({"op": "asset_diff", "seq": 2, "path": "card.png", "ref": "HEAD"}));
    let image = session.recv();
    assert_eq!(image["ok"], true, "mixed image review failed: {image}");
    assert_eq!(image["result"]["status"], "compared");
    assert!(
        image["result"]["changed_pixel_percentage"]
            .as_f64()
            .unwrap_or_default()
            > 0.0
    );
    drop(session.stdin);
    assert!(session.child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(base);
}

fn assert_incomplete_git_review(name: &str, old: &str, new: &str, offset: i64) {
    let wasm_dir =
        find_wasm_dir().expect("set INTENTUMDIFF_TEST_WASM_DIR to verified parser components");
    let bin = std::env::var("INTENTUMDIFF_TEST_LIVE_SERVER")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_intentumdiff-live-server").to_owned());
    {
        let repo = unique_temp_dir();
        git(&repo, &["init"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "T"]);
        std::fs::write(repo.join(name), old).unwrap();
        git(&repo, &["add", name]);
        git(&repo, &["commit", "-m", "incomplete baseline"]);
        std::fs::write(repo.join(name), new).unwrap();
        let mut session = spawn(&bin, &repo, &wasm_dir);
        assert_eq!(session.recv()["op"], "ready");
        session.send(&json!({"op": "review", "seq": 1, "old_ref": "HEAD", "stream": false}));
        let response = session.recv();
        assert_eq!(
            response["ok"], true,
            "{name} Git review must return source evidence: {response}"
        );
        let files = response["commit_diff"]["file_diffs"]
            .as_array()
            .expect("file diffs");
        let diff = files
            .iter()
            .find(|file| file["new_filename"] == name)
            .expect("incomplete file in review");
        assert_eq!(diff["metadata"]["engine_owner"], "rust");
        assert_eq!(
            diff["metadata"]["semantic_contract"],
            "rust_source_fallback_v1"
        );
        assert_eq!(diff["is_fallback"], true);
        assert_eq!(diff["is_style_only"], false);
        assert!(!diff["parse_errors"]
            .as_array()
            .expect("parse warnings")
            .is_empty());
        assert_eq!(diff["changes"].as_array().expect("changes").len(), 1);
        assert_eq!(diff["changes"][0]["old_node"]["label"], "f");
        assert_eq!(diff["changes"][0]["new_node"]["label"], "g");
        assert_eq!(
            diff["metadata"]["source_ranges"],
            json!({
                "old_start_byte": offset, "old_end_byte": offset + 1,
                "new_start_byte": offset, "new_end_byte": offset + 1,
            })
        );
        assert_eq!(diff["changes"][0]["old_node"]["position"]["start_line"], 0);
        assert_eq!(diff["changes"][0]["new_node"]["position"]["start_line"], 0);
        assert_eq!(
            diff["changes"][0]["old_node"]["position"]["start_col"],
            offset
        );
        assert_eq!(
            diff["changes"][0]["new_node"]["position"]["start_col"],
            offset
        );
        for side in ["old_node", "new_node"] {
            assert_eq!(diff["changes"][0][side]["position"]["end_line"], 0);
            assert_eq!(diff["changes"][0][side]["position"]["end_col"], offset + 1);
        }
        drop(session.stdin);
        assert!(session.child.wait().unwrap().success());
        let _ = std::fs::remove_dir_all(repo);
    }
}

#[test]
fn native_git_review_preserves_incomplete_python_evidence() {
    assert_incomplete_git_review("edit.py", "def f(", "def g(", 4);
}

#[test]
fn native_git_review_preserves_incomplete_javascript_evidence() {
    assert_incomplete_git_review("edit.js", "function f(", "function g(", 9);
}

fn assert_measured_parser_telemetry(
    diff: &Value,
    path: &str,
    language: &str,
    old: &str,
    new: &str,
) {
    let telemetry = &diff["metadata"]["engine_telemetry"];
    assert_eq!(
        telemetry["schema_version"], 1,
        "missing measured telemetry for {path}: {diff}"
    );
    let calls = telemetry["calls"]
        .as_array()
        .expect("engine telemetry calls");
    let processes: Vec<_> = calls
        .iter()
        .filter(|call| call["function"] == "process")
        .collect();
    assert!(
        !processes.is_empty(),
        "missing actual parser process measurements: {diff}"
    );
    let mut measured_sum = 0u64;
    let mut declared_total = 0u64;
    let mut peak = 0u64;
    for call in &processes {
        assert_eq!(call["engine_owner"], "rust");
        assert_eq!(call["engine"], "rust_wasmtime_plugin_host");
        assert_eq!(call["filename"], path);
        assert_eq!(call["language"], language);
        assert!(Path::new(call["plugin"].as_str().expect("actual parser path")).is_file());
        assert_eq!(call["call_count"], 1);
        assert_eq!(call["statuses"]["ok"], 1);
        let budget = call["fuel_budget"]
            .as_u64()
            .expect("actual metered fuel budget");
        let consumed = call["fuel_consumed"]
            .as_u64()
            .expect("actual consumed fuel");
        assert!(
            budget > 0 && consumed > 0 && consumed <= budget,
            "invalid fuel measurements: {call}"
        );
        let elapsed = call["elapsed_ms"].as_f64().expect("actual elapsed time");
        assert!(
            elapsed.is_finite() && elapsed >= 0.0,
            "invalid elapsed time: {call}"
        );
        let percent = call["max_fuel_used_percent"]
            .as_f64()
            .expect("measured fuel percentage");
        assert!((percent - consumed as f64 / budget as f64 * 100.0).abs() < 0.001);
        let bytes = call["input_bytes"].as_u64().expect("actual input bytes") as usize;
        assert!(
            bytes == old.len() || bytes == new.len(),
            "wrong parser input size: {call}"
        );
        assert_eq!(call["input_lines"], 1);
        measured_sum += consumed;
        declared_total += call["total_fuel_consumed"]
            .as_u64()
            .expect("actual total fuel");
        peak = peak.max(consumed);
    }
    for input in [old, new] {
        assert!(
            processes
                .iter()
                .any(|call| call["input_bytes"].as_u64() == Some(input.len() as u64)),
            "missing measurement for one source side: {diff}"
        );
    }
    assert_eq!(
        declared_total, measured_sum,
        "per-process total must match measured fuel"
    );
    assert!(declared_total >= peak && peak > 0);
}

fn check_native_parser_telemetry_fixture(name: &str, language: &str, old: &str, new: &str) {
    let wasm_dir =
        find_wasm_dir().expect("set INTENTUMDIFF_TEST_WASM_DIR to verified parser components");
    let bin = std::env::var("INTENTUMDIFF_TEST_LIVE_SERVER")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_intentumdiff-live-server").to_owned());
    let repo = unique_temp_dir();
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "T"]);
    let fixtures = [(name, language, old, new)];
    for (name, _, old, _) in fixtures {
        std::fs::write(repo.join(name), old).unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "telemetry baseline"]);
    for (name, _, _, new) in fixtures {
        std::fs::write(repo.join(name), new).unwrap();
    }
    let mut session = spawn(&bin, &repo, &wasm_dir);
    assert_eq!(session.recv()["op"], "ready");
    for (index, (name, language, old, new)) in fixtures.iter().enumerate() {
        session.send(&json!({"op":"diff", "seq":index + 1, "path":name, "content":new, "ref":"HEAD", "stream":false}));
        let response = session.recv();
        assert_eq!(response["ok"], true, "{response}");
        assert_measured_parser_telemetry(&response["diff"], name, language, old, new);
    }
    session.send(&json!({"op":"review", "seq":4, "old_ref":"HEAD", "stream":false}));
    let response = session.recv();
    assert_eq!(response["ok"], true, "{response}");
    let diffs = response["commit_diff"]["file_diffs"]
        .as_array()
        .expect("review diffs");
    for (name, language, old, new) in fixtures {
        let diff = diffs
            .iter()
            .find(|diff| diff["new_filename"] == name)
            .expect("review fixture");
        assert_measured_parser_telemetry(diff, name, language, old, new);
        if name == "broken.js" {
            assert_eq!(diff["is_fallback"], true);
            assert!(!diff["parse_errors"]
                .as_array()
                .expect("parse warnings")
                .is_empty());
            assert!(
                diff["metadata"]["engine_telemetry"]["calls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|call| call["function"] == "finalize" && call["fuel_consumed"].is_null()),
                "unmetered source-finalizer evidence must be preserved"
            );
        }
    }
    drop(session.stdin);
    assert!(session.child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn native_parser_telemetry_for_javascript() {
    check_native_parser_telemetry_fixture(
        "edit.js",
        "javascript",
        "function f(){return 1;}\n",
        "function f(){return 22;}\n",
    );
}

#[test]
fn native_parser_telemetry_for_json() {
    check_native_parser_telemetry_fixture(
        "settings.json",
        "json",
        "{\"count\":1}\n",
        "{\"count\":222}\n",
    );
}

#[test]
fn native_parser_telemetry_for_source_fallback() {
    check_native_parser_telemetry_fixture(
        "broken.js",
        "javascript",
        "function f(",
        "function longer(",
    );
}

#[test]
fn native_unlimited_live_diff_keeps_capped_measured_git_review() {
    let wasm_dir =
        find_wasm_dir().expect("set INTENTUMDIFF_TEST_WASM_DIR to verified parser components");
    let bin = std::env::var("INTENTUMDIFF_TEST_LIVE_SERVER")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_intentumdiff-live-server").to_owned());
    let repo = unique_temp_dir();
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "T"]);
    let old = "{\"count\":1}\n";
    let new = "{\"count\":22}\n";
    std::fs::write(repo.join("settings.json"), old).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "unlimited live baseline"]);
    std::fs::write(repo.join("settings.json"), new).unwrap();
    let mut session = spawn_with_extra_args(&bin, &repo, &wasm_dir, &["--fuel", "-1"]);
    assert_eq!(session.recv()["op"], "ready");
    session.send(&json!({"op":"review", "seq":1, "old_ref":"HEAD", "stream":false}));
    let review = session.recv();
    assert_eq!(review["ok"], true, "{review}");
    assert_eq!(
        review["metadata"]["review_fuel_capped"], true,
        "core review policy metadata was lost: {review}"
    );
    assert_eq!(review["metadata"]["review_plugin_fuel"], 100_000_000);
    let diff = &review["commit_diff"]["file_diffs"][0];
    assert_measured_parser_telemetry(diff, "settings.json", "json", old, new);
    for call in diff["metadata"]["engine_telemetry"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|call| call["function"] == "process")
    {
        assert_eq!(call["fuel_budget"], 100_000_000);
    }
    session.send(&json!({"op":"diff", "seq":2, "path":"settings.json", "content":new, "ref":"HEAD", "stream":false}));
    let direct = session.recv();
    assert_eq!(direct["ok"], true, "{direct}");
    let processes: Vec<_> = direct["diff"]["metadata"]["engine_telemetry"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|call| call["function"] == "process")
        .collect();
    assert!(
        !processes.is_empty(),
        "unlimited calls still need honest telemetry"
    );
    for call in processes {
        assert!(call["fuel_budget"].is_null());
        assert!(call["fuel_consumed"].is_null());
        assert!(call["total_fuel_consumed"].is_null());
        assert!(call["max_fuel_used_percent"].is_null());
        assert!(call["elapsed_ms"].as_f64().unwrap() >= 0.0);
    }
    drop(session.stdin);
    assert!(session.child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(repo);
}

#[test]
fn native_explicit_low_fuel_reports_parser_exhaustion() {
    let wasm_dir =
        find_wasm_dir().expect("set INTENTUMDIFF_TEST_WASM_DIR to verified parser components");
    let bin = std::env::var("INTENTUMDIFF_TEST_LIVE_SERVER")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_intentumdiff-live-server").to_owned());
    let repo = unique_temp_dir();
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "T"]);
    std::fs::write(repo.join("edit.js"), "function f(){return 1;}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "low fuel baseline"]);
    let mut session = spawn_with_extra_args(&bin, &repo, &wasm_dir, &["--fuel", "100"]);
    assert_eq!(session.recv()["op"], "ready");
    session.send(&json!({"op":"diff", "seq":1, "path":"edit.js",
        "content":"function f(){return 22;}\n", "ref":"HEAD", "stream":false}));
    let response = session.recv();
    assert_eq!(
        response["ok"], false,
        "explicit fuel limit must not be silently promoted: {response}"
    );
    assert!(
        response["error"]["message"]
            .as_str()
            .expect("structured error")
            .to_lowercase()
            .contains("fuel"),
        "{response}"
    );
    assert!(response.get("diff").is_none());
    drop(session.stdin);
    assert!(session.child.wait().unwrap().success());
    let _ = std::fs::remove_dir_all(repo);
}
