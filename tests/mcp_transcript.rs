//! Process-level tests for `renkin-mcp`: spawns the actual built binary and
//! drives it over real stdin/stdout pipes, the same way a real MCP client
//! would. Unit tests inside `src/mcp/*` exercise the protocol logic
//! in-process; these exist because that logic is only actually a
//! stdio-transport guarantee (§11: one message per line, stdout carries
//! only JSON-RPC, stderr separation, clean EOF shutdown) if the real
//! binary honors it too.

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

const LEGACY_INPUT: &str = include_str!("fixtures/mcp/2024-11-05/legacy_transcript_input.jsonl");
const LEGACY_GOLDEN_OUTPUT: &str =
    include_str!("fixtures/mcp/2024-11-05/legacy_transcript_output.jsonl");

fn bin() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_renkin-mcp"));
    cmd.current_dir(env!("CARGO_MANIFEST_DIR"));
    cmd
}

/// Runs `input` through the binary and returns (stdout, stderr, exit success).
/// Closes stdin after writing so the read loop hits EOF and the process
/// exits on its own, rather than needing to be killed.
fn run(input: &str) -> (String, String, bool) {
    let mut child = bin()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn renkin-mcp");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .expect("failed to write to child stdin");
    // stdin is dropped (closed) here, signalling EOF to the child.

    let output = child
        .wait_with_output()
        .expect("failed to wait for renkin-mcp");
    (
        String::from_utf8(output.stdout).expect("stdout was not valid UTF-8"),
        String::from_utf8(output.stderr).expect("stderr was not valid UTF-8"),
        output.status.success(),
    )
}

fn lines_as_json(s: &str) -> Vec<Value> {
    s.lines()
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("non-JSON stdout line {l:?}: {e}"))
        })
        .collect()
}

#[test]
fn legacy_transcript_stdout_matches_golden_fixture_structurally() {
    let (stdout, stderr, ok) = run(LEGACY_INPUT);
    assert!(ok, "process did not exit cleanly; stderr: {stderr}");
    assert!(
        stderr.is_empty(),
        "expected no stderr output, got: {stderr}"
    );

    let got = lines_as_json(&stdout);
    let want = lines_as_json(LEGACY_GOLDEN_OUTPUT);
    assert_eq!(got.len(), want.len(), "response count changed");
    for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
        // Deliberately normalized, not a structural deviation: the golden
        // fixture was captured against the pre-refactor binary at v0.18.0;
        // every release since (v0.19.0/v0.20.0/v0.21.0) legitimately moves
        // `serverInfo.version` (initialize, response #1) forward. Comparing
        // it verbatim would make this test fail at every future release
        // for a reason unrelated to protocol structure.
        if i == 0 {
            let mut g2 = g.clone();
            let mut w2 = w.clone();
            g2["result"]["serverInfo"]["version"] = Value::Null;
            w2["result"]["serverInfo"]["version"] = Value::Null;
            assert_eq!(
                g2,
                w2,
                "response #{} (minus serverInfo.version) diverged from the legacy golden transcript",
                i + 1
            );
            continue;
        }
        // The one deliberate, spec-mandated deviation: the stale "509
        // curated building blocks" claim was removed from find_routes'
        // description (tools/list, response #2). Every other legacy
        // response must be structurally identical to the pre-refactor binary.
        if i == 1 {
            let mut g2 = g.clone();
            let mut w2 = w.clone();
            g2["result"]["tools"][0]["description"] = Value::Null;
            w2["result"]["tools"][0]["description"] = Value::Null;
            assert_eq!(
                g2,
                w2,
                "response #{} (minus the description field) diverged from the legacy golden transcript",
                i + 1
            );
            continue;
        }
        assert_eq!(
            g,
            w,
            "response #{} diverged from the legacy golden transcript",
            i + 1
        );
    }
}

#[test]
fn legacy_transcript_every_stdout_line_is_standalone_valid_json() {
    let (stdout, _stderr, _ok) = run(LEGACY_INPUT);
    assert!(!stdout.is_empty());
    for line in stdout.lines() {
        serde_json::from_str::<Value>(line)
            .unwrap_or_else(|e| panic!("line failed to parse as JSON: {line:?}: {e}"));
    }
}

#[test]
fn modern_transcript_discover_then_tools_list_then_tools_call() {
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":"d1","method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"test-client","version":"1.0.0"},"io.modelcontextprotocol/clientCapabilities":{}}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":"t1","method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":"c1","method":"tools/call","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}},"name":"find_routes","arguments":{"smiles":"CCO","depth":2,"max_routes":1}}}"#,
        "\n",
    );
    let (stdout, stderr, ok) = run(input);
    assert!(ok, "process did not exit cleanly; stderr: {stderr}");
    assert!(
        stderr.is_empty(),
        "expected no stderr output, got: {stderr}"
    );

    let resp = lines_as_json(&stdout);
    assert_eq!(resp.len(), 3);

    let discover = &resp[0];
    assert_eq!(discover["id"], "d1");
    assert_eq!(discover["result"]["resultType"], "complete");
    assert_eq!(
        discover["result"]["supportedVersions"],
        serde_json::json!(["2026-07-28"])
    );
    assert_eq!(
        discover["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "renkin"
    );

    let tools_list = &resp[1];
    assert_eq!(tools_list["id"], "t1");
    assert_eq!(tools_list["result"]["resultType"], "complete");
    assert!(tools_list["result"]["ttlMs"].as_u64().unwrap() > 0);
    assert_eq!(tools_list["result"]["cacheScope"], "public");
    assert!(tools_list["result"]["tools"].as_array().unwrap().len() >= 7);

    let tools_call = &resp[2];
    assert_eq!(tools_call["id"], "c1");
    assert_eq!(tools_call["result"]["resultType"], "complete");
    assert!(tools_call["result"]["content"].is_array());
}

#[test]
fn malformed_json_gets_a_parse_error_response_not_silence() {
    let (stdout, _stderr, ok) = run("{not json\n");
    assert!(ok);
    let resp = lines_as_json(&stdout);
    assert_eq!(resp.len(), 1);
    assert_eq!(resp[0]["error"]["code"], -32700);
    assert_eq!(resp[0]["id"], Value::Null);
}

#[test]
fn unknown_method_on_a_fresh_connection_is_method_not_found_and_does_not_pin() {
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"totally/unknown\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"c\",\"version\":\"1\"}}}\n",
    );
    let (stdout, _stderr, ok) = run(input);
    assert!(ok);
    let resp = lines_as_json(&stdout);
    assert_eq!(resp.len(), 2);
    assert_eq!(resp[0]["error"]["code"], -32601);
    // A still-Unpinned connection must accept a later `initialize`.
    assert_eq!(resp[1]["result"]["protocolVersion"], "2024-11-05");
}

#[test]
fn notifications_produce_no_output_at_all() {
    let input = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
    let (stdout, stderr, ok) = run(input);
    assert!(ok);
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

#[test]
fn clean_eof_shutdown_within_a_reasonable_time() {
    let start = std::time::Instant::now();
    let (_stdout, _stderr, ok) = run("");
    assert!(ok);
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "process took too long to exit on EOF with no input"
    );
}
