//! Integration tests for the MCP server.
//!
//! Spawns `omd mcp` as a child process with an isolated OMD_HOME pointing at
//! an httpmock server and drives the server via newline-delimited JSON-RPC
//! over stdin/stdout.

use httpmock::prelude::*;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Once;
use std::time::Duration;

static BUILD_ONCE: Once = Once::new();

fn binary() -> std::path::PathBuf {
    BUILD_ONCE.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "--bin", "omd"])
            .status()
            .expect("cargo build");
        assert!(status.success(), "cargo build failed");
    });
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target");
    p.push("debug");
    p.push("omd");
    p
}

struct Session {
    child: std::process::Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    _home: tempfile::TempDir,
}

impl Session {
    fn start(host: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::write(
            home.join("config.toml"),
            format!("[profiles.default]\nhost = \"{host}\"\ntimeout_secs = 5\n"),
        )
        .unwrap();
        std::fs::write(
            home.join("credentials"),
            "[tokens]\ndefault = \"test-token\"\n",
        )
        .unwrap();

        let mut child = Command::new(binary())
            .arg("mcp")
            .env("OMD_HOME", home)
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn omd mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Self {
            child,
            stdin,
            stdout,
            _home: dir,
        };
        s.initialize();
        s
    }

    fn send(&mut self, msg: &Value) {
        let line = serde_json::to_string(msg).unwrap();
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read");
        serde_json::from_str(&line).expect("parse jsonrpc")
    }

    fn initialize(&mut self) {
        self.send(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }
        }));
        let resp = self.recv();
        assert!(resp.get("result").is_some(), "init failed: {resp}");
        self.send(&json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        }));
    }

    fn call_tool(&mut self, id: u32, name: &str, args: Value) -> Value {
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": args }
        }));
        self.recv()
    }

    fn list_tools(&mut self, id: u32) -> Vec<String> {
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {}
        }));
        let resp = self.recv();
        resp.get("result")
            .and_then(|r| r.get("tools"))
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool_text(resp: &Value) -> String {
    resp.get("result")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .to_string()
}

#[test]
fn tools_list_returns_curated_set() {
    let server = MockServer::start();
    let mut s = Session::start(&server.base_url());
    let tools = s.list_tools(2);
    assert!(
        tools.len() >= 14,
        "expected at least 14 tools, got {}",
        tools.len()
    );
    for required in &[
        "search",
        "describe_entity",
        "resolve_fqn",
        "get_lineage",
        "list_upstream",
        "list_downstream",
        "update_description",
        "add_tag",
        "remove_tag",
        "assign_glossary_term",
        "list_quality_tests",
        "get_test_results",
        "export_csv",
        "import_csv",
    ] {
        assert!(
            tools.contains(&required.to_string()),
            "missing tool: {required}"
        );
    }
}

#[test]
fn search_tool_hits_search_api() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/search/query")
            .query_param("q", "orders")
            .query_param("size", "3");
        then.status(200)
            .json_body(json!({ "hits": { "hits": [{"_source": {"name": "orders"}}] }}));
    });
    let mut s = Session::start(&server.base_url());
    let resp = s.call_tool(10, "search", json!({ "query": "orders", "limit": 3 }));
    assert!(resp.get("result").is_some(), "tool call failed: {resp}");
    mock.assert();
    let text = tool_text(&resp);
    assert!(
        text.contains("orders"),
        "expected orders in content: {text}"
    );
}

#[test]
fn raw_request_is_gated_behind_env() {
    let server = MockServer::start();
    let mut s = Session::start(&server.base_url());
    let resp = s.call_tool(
        20,
        "raw_request",
        json!({ "method": "GET", "path": "v1/whatever" }),
    );
    // With OMD_MCP_ALLOW_RAW unset, we expect a jsonrpc error.
    assert!(
        resp.get("error").is_some(),
        "expected error for raw_request without allow-raw env: {resp}"
    );
    let msg = resp
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or_default();
    assert!(msg.contains("raw_request disabled"), "got: {msg}");
}

#[test]
fn import_csv_dry_run_by_default() {
    let server = MockServer::start();
    let hit = server.mock(|when, then| {
        when.method(PUT)
            .path("/api/v1/tables/name/svc.db.s.t/import")
            .query_param("dryRun", "true")
            .header("content-type", "text/plain")
            .body("a,b\n1,2\n");
        then.status(200).json_body(json!({
            "dryRun": true,
            "status": "success",
            "numberOfRowsProcessed": 1,
            "numberOfRowsPassed": 1,
            "numberOfRowsFailed": 0
        }));
    });
    let mut s = Session::start(&server.base_url());
    let resp = s.call_tool(
        30,
        "import_csv",
        json!({
            "entity_type": "table",
            "fqn": "svc.db.s.t",
            "csv": "a,b\n1,2\n"
        }),
    );
    hit.assert();
    let text = tool_text(&resp);
    assert!(text.contains("\"status\": \"success\""), "got: {text}");
}

#[test]
fn give_server_a_moment_to_exit_cleanly() {
    // Keeps a sanity handle on the binary path to avoid confusing failures.
    std::thread::sleep(Duration::from_millis(10));
    assert!(binary().exists());
}
