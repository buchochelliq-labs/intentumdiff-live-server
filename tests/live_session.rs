//! End-to-end regression test for the native live-server (#100): spawn the built binary and
//! drive a full JSON-line protocol-v2 session over stdio, asserting the wire contract the VS
//! Code extension depends on — `ready`, `hello`, `diff`, `review`, `cancel`, clean EOF exit.
//! Self-contained (shells out to `git`, no Python); skips (passes with a note) when the
//! bundled wasm parsers or `git` are unavailable.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

/// The dev-layout wasm dir (walk ancestors for `src/intentumdiff/wasm`), manifest-verified.
fn find_wasm_dir() -> Option<PathBuf> {
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
    let dir =
        std::env::temp_dir().join(format!("intentumdiff-live-it-{}-{nanos}", std::process::id()));
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
    let Some(wasm_dir) = find_wasm_dir() else {
        eprintln!("skipping: bundled wasm parsers not built (no parser_manifest.json)");
        return;
    };
    if !git_available() {
        eprintln!("skipping: git not available");
        return;
    }
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
    assert!(ready["wasm_dir"].is_string(), "wasm_dir should be resolved: {ready}");
    assert!(
        ready["capabilities"]["review"].as_bool().unwrap_or(false),
        "review capability should be advertised: {ready}"
    );

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
    assert!(change_types.contains(&"MODIFICATION"), "got {change_types:?}");

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

    // 6. Clean shutdown on stdin EOF (#73).
    drop(s.stdin);
    let status = s.child.wait().expect("wait");
    assert!(status.success(), "clean exit expected, got {status:?}");

    let _ = std::fs::remove_dir_all(&base);
}
