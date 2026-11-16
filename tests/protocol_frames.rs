//! Transport-level protocol tests that need neither parser components nor Git: the hello
//! handshake, frame decoding and cancellation semantics of the stdio transport. Each test
//! writes raw bytes to the real binary's stdin, closes it, and inspects every response line
//! plus the exit status.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

fn temp_repo() -> PathBuf {
    // Parallel tests: a coarse clock can repeat a timestamp, so add a per-process counter
    // and refuse an existing directory (see live_session.rs unique_temp_dir).
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "intentumdiff-live-frames-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).expect("unique temp dir");
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
    assert_eq!(ready["server_version"], env!("CARGO_PKG_VERSION"), "{ready}");
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

// --- frame decoding (#33): invalid UTF-8 is an isolated bad frame, not EOF ---

fn hello_line(seq: i64) -> Vec<u8> {
    line(json!({"op": "hello", "seq": seq}))
}

fn assert_invalid_utf8_then_hello(frame: &[u8]) {
    let mut input = frame.to_vec();
    input.extend(hello_line(9));
    let (responses, output) = session(&input);
    assert!(output.status.success(), "clean EOF must exit 0: {:?}", output.status);
    assert_eq!(responses.len(), 2, "{responses:?}");
    assert_eq!(responses[0]["ok"], false, "{responses:?}");
    assert_eq!(responses[0]["error"]["code"], "invalid_utf8", "{responses:?}");
    assert_eq!(responses[1]["op"], "hello", "{responses:?}");
    assert_eq!(responses[1]["seq"], 9);
    assert_eq!(responses[1]["ok"], true);
}

#[test]
fn invalid_utf8_frame_gets_one_error_and_the_session_recovers() {
    assert_invalid_utf8_then_hello(b"\xff\n");
}

#[test]
fn invalid_utf8_inside_a_json_string_gets_one_error_and_the_session_recovers() {
    assert_invalid_utf8_then_hello(b"{\"op\":\"hello\",\"seq\":1,\"x\":\"a\xffb\"}\n");
}

#[test]
fn invalid_utf8_with_crlf_and_truncated_sequence_recovers() {
    assert_invalid_utf8_then_hello(b"\xe2\x82\r\n");
}

#[test]
fn oversized_frame_is_rejected_without_losing_the_next_request() {
    let mut input = vec![b'a'; 1_048_576 + 10];
    input.push(b'\n');
    input.extend(hello_line(10));
    let (responses, output) = session(&input);
    assert!(output.status.success());
    assert_eq!(responses.len(), 2, "{responses:?}");
    assert_eq!(responses[0]["error"]["code"], "line_too_large");
    assert_eq!(responses[1]["seq"], 10);
    assert_eq!(responses[1]["ok"], true);
}

#[test]
fn final_frame_without_newline_is_still_served_before_clean_eof() {
    let (responses, output) = session(br#"{"op":"hello","seq":11}"#);
    assert!(output.status.success());
    assert_eq!(responses.len(), 1, "{responses:?}");
    assert_eq!(responses[0]["seq"], 11);
}

#[cfg(target_os = "linux")]
#[test]
fn stdin_read_failure_is_reported_and_distinct_from_clean_eof() {
    // Reading a directory fd fails with EISDIR: a genuine I/O error, not end of input.
    let repo = temp_repo();
    let output = Command::new(env!("CARGO_BIN_EXE_intentumdiff-live-server"))
        .arg("--repo")
        .arg(&repo)
        .stdin(std::fs::File::open(&repo).expect("open dir"))
        .output()
        .expect("run");
    let _ = std::fs::remove_dir_all(&repo);
    assert!(!output.status.success(), "read failure must not look like clean EOF");
    let text = String::from_utf8(output.stdout).unwrap();
    let last: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(last["error"]["code"], "io_error", "{text}");
    // Clean EOF, by contrast, exits 0 with no error line.
    let (responses, output) = session(b"");
    assert!(output.status.success());
    assert!(responses.is_empty(), "{responses:?}");
}

// --- cancellation (#34): the synchronous transport must not imply it can interrupt ---

#[test]
fn capabilities_state_that_cancel_does_not_interrupt_in_flight_work() {
    let (responses, _) = session(&hello_line(1));
    let hello = &responses[0];
    assert_eq!(hello["capabilities"]["cancel_interrupts"], false, "{hello}");
    // The cancel op itself stays available (a no-op acknowledgement).
    assert!(hello["capabilities"]["operations"]
        .as_array()
        .unwrap()
        .contains(&json!("cancel")));
}

#[test]
fn cancel_acknowledges_without_claiming_an_interrupt_and_the_session_continues() {
    let mut input = Vec::new();
    // A request that completes (here: fails fast) before the cancel is read...
    input.extend(line(json!({"op": "diff", "seq": 5, "path": "a.py", "content": "x = 1\n"})));
    // ...its cancel (already completed), a cancel for an unknown target, a path cancel...
    input.extend(line(json!({"op": "cancel", "seq": 5})));
    input.extend(line(json!({"op": "cancel", "seq": 424242})));
    input.extend(line(json!({"op": "cancel", "seq": 6, "path": "a.py"})));
    // ...and an immediate next request.
    input.extend(hello_line(7));
    let (responses, output) = session(&input);
    assert!(output.status.success());
    assert_eq!(responses.len(), 5, "{responses:?}");
    assert_eq!(responses[0]["op"], "diff");
    assert_eq!(responses[0]["seq"], 5);
    for (cancel, seq) in responses[1..4].iter().zip([5, 424242, 6]) {
        assert_eq!(cancel["op"], "cancel", "{cancel}");
        assert_eq!(cancel["seq"], seq, "{cancel}");
        assert_eq!(cancel["ok"], true, "{cancel}");
        assert_eq!(cancel["cancelled"], 0, "{cancel}");
        assert_eq!(cancel["interrupted"], false, "{cancel}");
    }
    assert_eq!(responses[4]["op"], "hello");
    assert_eq!(responses[4]["seq"], 7);
    assert_eq!(responses[4]["ok"], true);
}
