//! Integration tests for dynamic OpenAPI-driven commands.
//!
//! Spins up httpmock as a fake OpenMetadata server, points a temp OMD_HOME
//! at a minimal spec + credentials, and invokes the real `omd` binary to
//! verify end-to-end behavior.

use httpmock::prelude::*;
use std::process::Command;
use std::sync::Once;

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

struct TempHome {
    dir: tempfile::TempDir,
}

impl TempHome {
    fn new(host: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::write(
            home.join("config.toml"),
            format!("[profiles.default]\nhost = \"{host}\"\ntimeout_secs = 5\n",),
        )
        .unwrap();
        std::fs::write(
            home.join("credentials"),
            "[tokens]\ndefault = \"test-token\"\n",
        )
        .unwrap();
        let spec = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini-spec.json"),
        )
        .unwrap();
        std::fs::write(home.join("spec.json"), spec).unwrap();
        Self { dir }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(binary());
        c.env("OMD_HOME", self.dir.path());
        c.env("NO_COLOR", "1");
        c.arg("--json");
        for a in args {
            c.arg(a);
        }
        c
    }
}

fn stdout_str(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn dynamic_list_hits_expected_endpoint() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/tables")
            .query_param("limit", "5")
            .header("authorization", "Bearer test-token");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"data":[{"id":"t1","name":"tbl"}]}"#);
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&["tables", "list", "--limit", "5"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        stdout_str(&out),
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert();
    assert!(stdout_str(&out).contains("\"tbl\""));
}

#[test]
fn dynamic_path_param_substituted() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/tables/abc-123")
            .query_param("fields", "columns");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"abc-123","name":"users"}"#);
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&["tables", "get-by-id", "abc-123", "--fields", "columns"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert();
}

#[test]
fn dynamic_body_inline_json() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/api/v1/tables")
            .header("content-type", "application/json")
            .json_body(serde_json::json!({"name": "orders"}));
        then.status(201)
            .header("content-type", "application/json")
            .body(r#"{"id":"new","name":"orders"}"#);
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&["tables", "create", "--body", r#"{"name":"orders"}"#])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert();
}

#[test]
fn dynamic_body_from_file() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/api/v1/tables")
            .json_body(serde_json::json!({"name": "from-file"}));
        then.status(200).body(r#"{"ok":true}"#);
    });
    let home = TempHome::new(&server.base_url());
    let body_file = home.dir.path().join("body.json");
    std::fs::write(&body_file, r#"{"name":"from-file"}"#).unwrap();
    let body_arg = format!("@{}", body_file.display());
    let out = home
        .cmd(&["tables", "create", "--body", &body_arg])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert();
}

#[test]
fn dynamic_unknown_group_reports_available() {
    let home = TempHome::new("http://127.0.0.1:1");
    let out = home.cmd(&["nonsense-group", "list"]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown group"), "got: {err}");
    assert!(
        err.contains("tables"),
        "should list available groups: {err}"
    );
}

#[test]
fn dynamic_help_lists_actions() {
    let home = TempHome::new("http://127.0.0.1:1");
    let out = home.cmd(&["tables", "--help"]).output().unwrap();
    let s = stdout_str(&out);
    assert!(s.contains("list"), "help should list `list` action: {s}");
    assert!(s.contains("get-by-id"), "help should list `get-by-id`: {s}");
    assert!(s.contains("create"), "help should list `create`: {s}");
}
