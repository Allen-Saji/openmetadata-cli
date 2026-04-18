//! Integration tests for the CSV import/export commands.

use httpmock::prelude::*;
use httpmock::Method::PUT;
use serde_json::json;
use std::process::{Command, Stdio};
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
            format!("[profiles.default]\nhost = \"{host}\"\ntimeout_secs = 5\n"),
        )
        .unwrap();
        std::fs::write(
            home.join("credentials"),
            "[tokens]\ndefault = \"test-token\"\n",
        )
        .unwrap();
        Self { dir }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(binary());
        c.env("OMD_HOME", self.dir.path());
        c.env("NO_COLOR", "1");
        for a in args {
            c.arg(a);
        }
        c
    }
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ------- export -------

#[test]
fn export_prints_csv_to_stdout() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/tables/name/svc.db.s.orders/export");
        then.status(200)
            .json_body(json!("name,description\norders,order table\n"));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&["export", "table", "svc.db.s.orders"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = stdout(&out);
    assert!(s.starts_with("name,description"));
    assert!(s.contains("orders,order table"));
}

#[test]
fn export_writes_csv_to_file() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/glossaries/name/Retail/export");
        then.status(200)
            .json_body(json!("term,description\nSku,A unit\n"));
    });
    let home = TempHome::new(&server.base_url());
    let out_path = home.dir.path().join("retail.csv");
    let out = home
        .cmd(&[
            "export",
            "glossary",
            "Retail",
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let body = std::fs::read_to_string(&out_path).unwrap();
    assert!(body.contains("term,description"));
    assert!(body.contains("Sku"));
}

#[test]
fn export_unsupported_type_errors() {
    let home = TempHome::new("http://127.0.0.1:1");
    let out = home.cmd(&["export", "pipeline", "x"]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("does not support CSV"), "got: {err}");
    assert!(err.contains("table"), "should list supported: {err}");
}

// ------- import -------

#[test]
fn import_defaults_to_dry_run() {
    let server = MockServer::start();
    let hit = server.mock(|when, then| {
        when.method(PUT)
            .path("/api/v1/tables/name/svc.db.s.orders/import")
            .query_param("dryRun", "true")
            .header("content-type", "text/plain")
            .body("name,description\norders,new description\n");
        then.status(200).json_body(json!({
            "dryRun": true,
            "status": "success",
            "numberOfRowsProcessed": 1,
            "numberOfRowsPassed": 1,
            "numberOfRowsFailed": 0
        }));
    });
    let home = TempHome::new(&server.base_url());
    let csv_path = home.dir.path().join("update.csv");
    std::fs::write(&csv_path, "name,description\norders,new description\n").unwrap();
    let out = home
        .cmd(&[
            "import",
            "table",
            "svc.db.s.orders",
            csv_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    hit.assert();
    assert!(stdout(&out).contains("\"status\": \"success\""));
}

#[test]
fn import_apply_flips_dry_run_off() {
    let server = MockServer::start();
    let hit = server.mock(|when, then| {
        when.method(PUT)
            .path("/api/v1/tables/name/svc.db.s.orders/import")
            .query_param("dryRun", "false");
        then.status(200).json_body(json!({
            "dryRun": false,
            "status": "success",
            "numberOfRowsProcessed": 2,
            "numberOfRowsPassed": 2,
            "numberOfRowsFailed": 0
        }));
    });
    let home = TempHome::new(&server.base_url());
    let csv_path = home.dir.path().join("update.csv");
    std::fs::write(&csv_path, "a,b\n1,2\n3,4\n").unwrap();
    let out = home
        .cmd(&[
            "import",
            "table",
            "svc.db.s.orders",
            csv_path.to_str().unwrap(),
            "--apply",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    hit.assert();
}

#[test]
fn import_failure_status_exits_nonzero() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(PUT)
            .path("/api/v1/tables/name/svc.db.s.orders/import");
        then.status(200).json_body(json!({
            "dryRun": true,
            "status": "failure",
            "numberOfRowsProcessed": 1,
            "numberOfRowsPassed": 0,
            "numberOfRowsFailed": 1,
            "abortReason": "column mismatch"
        }));
    });
    let home = TempHome::new(&server.base_url());
    let csv_path = home.dir.path().join("bad.csv");
    std::fs::write(&csv_path, "a,b\n1,\n").unwrap();
    let out = home
        .cmd(&[
            "import",
            "table",
            "svc.db.s.orders",
            csv_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "should exit nonzero on failure status"
    );
}

#[test]
fn import_reads_stdin_with_dash() {
    let server = MockServer::start();
    let hit = server.mock(|when, then| {
        when.method(PUT)
            .path("/api/v1/tables/name/svc.db.s.orders/import")
            .body("name,description\nfrom_stdin,value\n");
        then.status(200).json_body(json!({
            "dryRun": true,
            "status": "success",
            "numberOfRowsProcessed": 1,
            "numberOfRowsPassed": 1,
            "numberOfRowsFailed": 0
        }));
    });
    let home = TempHome::new(&server.base_url());
    let mut child = home
        .cmd(&["import", "table", "svc.db.s.orders", "-", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin
            .write_all(b"name,description\nfrom_stdin,value\n")
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    hit.assert();
}
