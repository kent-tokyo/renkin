use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

fn run_mcp(input: &str) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_renkin-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("renkin-mcp must spawn");
    child
        .stdin
        .take()
        .expect("stdin must be available")
        .write_all(input.as_bytes())
        .expect("request input must be written");
    let output = child.wait_with_output().expect("server must exit");
    assert!(
        output.status.success(),
        "server failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("stdout must be UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each response must be JSON"))
        .collect()
}

#[test]
fn invalid_json_rpc_envelope_is_rejected_before_dispatch_and_next_frame_survives() {
    let responses = run_mcp(
        "{\"id\":1,\"method\":\"tools/list\"}\n\
         {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
    );
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], -32600);
    assert_eq!(responses[0]["id"], Value::Null);
    assert_eq!(responses[1]["id"], 2);
    assert!(responses[1]["result"]["tools"].is_array());
}

#[test]
fn ordinary_notification_executes_without_emitting_a_response() {
    let responses = run_mcp(
        "{\"jsonrpc\":\"2.0\",\"method\":\"tools/list\"}\n\
         {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n",
    );
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 1);
    assert!(responses[0]["result"]["tools"].is_array());
}
