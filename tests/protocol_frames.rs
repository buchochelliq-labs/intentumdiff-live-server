//! Transport-level protocol tests that need neither parser components nor Git: the hello
//! handshake, frame decoding and cancellation semantics of the stdio transport. Each test
//! writes raw bytes to the real binary's stdin, closes it, and inspects every response line
//! plus the exit status.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

fn temp_repo() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "intentumdiff-live-frames-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Run one session: write `input` verbatim, close stdin, return (responses after ready, output).
fn session(input: &[u8]) -> (Vec<Value>, Output) {
    let repo = temp_repo();
    let mut child = Command::new(env!("CARGO_BIN_EXE_intentumdiff-live-server"))
        .arg("--repo")
        .arg(&repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn live-server");
    let mut stdin = child.stdin.take().expect("stdin");
    let bytes = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
        // Dropping stdin delivers EOF.
    });
    let output = child.wait_with_output().expect("wait");
    writer.join().unwrap();
    let _ = std::fs::remove_dir_all(repo);
    let text = String::from_utf8(output.stdout.clone()).expect("responses are UTF-8");
    let mut lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
        .collect();
    assert!(!lines.is_empty(), "no ready line");
    let ready = lines.remove(0);
    assert_eq!(ready["op"], "ready", "{ready}");
    assert_eq!(ready["ok"], true, "{ready}");
    assert_eq!(ready["protocol_version"], 2, "{ready}");
    assert_eq!(ready["supported_protocol_versions"], json!([2]), "{ready}");
    (lines, output)
}

fn line(value: Value) -> Vec<u8> {
    let mut bytes = value.to_string().into_bytes();
    bytes.push(b'\n');
    bytes
}

fn single(input: Value) -> Value {
    let (responses, output) = session(&line(input));
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(responses.len(), 1, "{responses:?}");
    responses.into_iter().next().unwrap()
}

// --- hello: protocol negotiation is Core's live_negotiate_protocol (core #260) ---

#[test]
fn hello_accepts_supported_version_and_reports_supported_set() {
    let hello = single(json!({"op": "hello", "seq": 1, "protocol_version": 2}));
    assert_eq!(hello["op"], "hello");
    assert_eq!(hello["seq"], 1);
    assert_eq!(hello["ok"], true, "{hello}");
    assert_eq!(hello["protocol_version"], 2);
    assert_eq!(hello["supported_protocol_versions"], json!([2]));
    // Transport context is still provided on success.
    assert!(hello["limits"].is_object(), "{hello}");
    assert!(hello["capabilities"].is_object(), "{hello}");
    assert!(hello["repo_path"].is_string(), "{hello}");
}

#[test]
fn hello_without_or_with_null_version_takes_current_version() {
    for request in [
        json!({"op": "hello", "seq": 2}),
        json!({"op": "hello", "seq": 2, "protocol_version": null}),
    ] {
        let hello = single(request.clone());
        assert_eq!(hello["ok"], true, "{request}: {hello}");
        assert_eq!(hello["seq"], 2);
        assert_eq!(hello["protocol_version"], 2);
        assert_eq!(hello["supported_protocol_versions"], json!([2]));
    }
}

#[test]
fn hello_rejects_unsupported_integer_version() {
    for version in [json!(1), json!(3), json!(999)] {
        let hello = single(json!({"op": "hello", "seq": 3, "protocol_version": version}));
        assert_eq!(hello["op"], "hello", "{hello}");
        assert_eq!(hello["seq"], 3);
        assert_eq!(hello["ok"], false, "{version} must not be accepted: {hello}");
        assert_eq!(hello["error"]["code"], "unsupported_protocol_version");
        assert_eq!(hello["error"]["supported_protocol_versions"], json!([2]));
        assert!(hello.get("protocol_version").is_none(), "{hello}");
    }
}

#[test]
fn hello_rejects_non_integer_version() {
    for version in [json!("2"), json!(2.5), json!(-1), json!(true), json!([2]), json!({})] {
        let hello = single(json!({"op": "hello", "seq": 4, "protocol_version": version}));
        assert_eq!(hello["ok"], false, "{version}: {hello}");
        assert_eq!(hello["seq"], 4);
        assert_eq!(hello["error"]["code"], "invalid_protocol_version", "{version}: {hello}");
        assert_eq!(hello["error"]["supported_protocol_versions"], json!([2]));
    }
}

#[test]
fn hello_uses_the_transport_coerced_seq() {
    // seq coercion is the shared live_request_seq rule; negotiation must echo the same seq.
    let hello = single(json!({"op": "hello", "seq": "7", "protocol_version": 999}));
    assert_eq!(hello["seq"], 7, "{hello}");
    assert_eq!(hello["error"]["code"], "unsupported_protocol_version");
}
