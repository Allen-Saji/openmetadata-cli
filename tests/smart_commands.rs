//! Integration tests for v0.3 smart commands.

use httpmock::prelude::*;
use httpmock::Method::PATCH;
use serde_json::{json, Value};
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

fn search_hit(fqn: &str, entity_type: &str, id: &str) -> Value {
    json!({
        "hits": {
            "hits": [
                { "_source": {
                    "id": id,
                    "fullyQualifiedName": fqn,
                    "entityType": entity_type
                }}
            ]
        }
    })
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ------- lineage -------

#[test]
fn lineage_tree_renders() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/search/query")
            .query_param("q", "fullyQualifiedName:\"svc.db.s.orders\"");
        then.status(200)
            .json_body(search_hit("svc.db.s.orders", "table", "root-id"));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/lineage/table/name/svc.db.s.orders")
            .query_param("upstreamDepth", "2")
            .query_param("downstreamDepth", "2");
        then.status(200).json_body(json!({
            "entity": {
                "id": "root-id",
                "fullyQualifiedName": "svc.db.s.orders",
                "type": "table"
            },
            "nodes": [
                { "id": "up-1", "fullyQualifiedName": "svc.db.s.raw", "type": "table" },
                { "id": "down-1", "fullyQualifiedName": "svc.db.s.summary", "type": "table" }
            ],
            "upstreamEdges": [{ "fromEntity": "up-1", "toEntity": "root-id" }],
            "downstreamEdges": [{ "fromEntity": "root-id", "toEntity": "down-1" }]
        }));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&["lineage", "svc.db.s.orders", "--format", "tree"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = stdout(&out);
    assert!(s.contains("svc.db.s.orders"), "root missing: {s}");
    assert!(s.contains("svc.db.s.raw"), "upstream missing: {s}");
    assert!(s.contains("svc.db.s.summary"), "downstream missing: {s}");
}

#[test]
fn lineage_mermaid_format() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/search/query");
        then.status(200)
            .json_body(search_hit("svc.db.s.orders", "table", "root-id"));
    });
    server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/lineage/table/name/svc.db.s.orders");
        then.status(200).json_body(json!({
            "entity": { "id": "root-id", "fullyQualifiedName": "svc.db.s.orders", "type": "table" },
            "nodes": [
                { "id": "down-1", "fullyQualifiedName": "svc.db.s.summary", "type": "table" }
            ],
            "upstreamEdges": [],
            "downstreamEdges": [{ "fromEntity": "root-id", "toEntity": "down-1" }]
        }));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&["lineage", "svc.db.s.orders", "--format", "mermaid"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = stdout(&out);
    assert!(s.starts_with("graph LR"), "expected mermaid: {s}");
    assert!(s.contains("-->"));
}

// ------- edit -------

#[test]
fn edit_description_patch_shape() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/search/query");
        then.status(200)
            .json_body(search_hit("svc.db.s.orders", "table", "table-id-1"));
    });
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/tables/name/svc.db.s.orders");
        then.status(200).json_body(json!({
            "id": "table-id-1",
            "name": "orders",
            "description": "old",
            "tags": []
        }));
    });
    let patch_hit = server.mock(|when, then| {
        when.method(PATCH)
            .path("/api/v1/tables/table-id-1")
            .header("content-type", "application/json-patch+json")
            .json_body(json!([
                { "op": "replace", "path": "/description", "value": "new desc" }
            ]));
        then.status(200).json_body(json!({
            "id": "table-id-1",
            "description": "new desc"
        }));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&[
            "edit",
            "svc.db.s.orders",
            "--description",
            "new desc",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    patch_hit.assert();
}

#[test]
fn edit_dry_run_emits_patch() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/search/query");
        then.status(200)
            .json_body(search_hit("svc.db.s.orders", "table", "t1"));
    });
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/tables/name/svc.db.s.orders");
        then.status(200).json_body(json!({"id": "t1", "tags": []}));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&[
            "edit",
            "svc.db.s.orders",
            "--display-name",
            "Orders",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = stdout(&out);
    assert!(s.contains("/displayName"), "missing path: {s}");
    assert!(s.contains("\"Orders\""), "missing value: {s}");
}

// ------- tag -------

#[test]
fn tag_add_and_remove_patch() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/search/query");
        then.status(200)
            .json_body(search_hit("svc.db.s.orders", "table", "t1"));
    });
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/tables/name/svc.db.s.orders");
        then.status(200).json_body(json!({
            "id": "t1",
            "tags": [
                { "tagFQN": "PII.Sensitive", "source": "Classification" }
            ]
        }));
    });
    let patch_hit = server.mock(|when, then| {
        when.method(PATCH)
            .path("/api/v1/tables/t1")
            .header("content-type", "application/json-patch+json");
        then.status(200).json_body(json!({"id":"t1"}));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&[
            "tag",
            "svc.db.s.orders",
            "--add",
            "Tier.Tier2",
            "--remove",
            "PII.Sensitive",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    patch_hit.assert();
}

#[test]
fn tag_dry_run_no_http_call() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/search/query");
        then.status(200)
            .json_body(search_hit("svc.db.s.orders", "table", "t1"));
    });
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/tables/name/svc.db.s.orders");
        then.status(200).json_body(json!({"id":"t1","tags":[]}));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&["tag", "svc.db.s.orders", "--add", "Tier.Tier1", "--dry-run"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = stdout(&out);
    assert!(s.contains("Tier.Tier1"));
    assert!(s.contains("Classification"));
}

// ------- glossary -------

#[test]
fn glossary_assign_patches_tags_with_glossary_source() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/search/query");
        then.status(200)
            .json_body(search_hit("svc.db.s.orders", "table", "t1"));
    });
    server.mock(|when, then| {
        when.method(GET).path("/api/v1/tables/name/svc.db.s.orders");
        then.status(200).json_body(json!({"id":"t1","tags":[]}));
    });
    let patch_hit = server.mock(|when, then| {
        when.method(PATCH).path("/api/v1/tables/t1").matches(|req| {
            let body = req.body.as_deref().unwrap_or_default();
            let s = std::str::from_utf8(body).unwrap_or_default();
            s.contains("\"source\":\"Glossary\"") && s.contains("\"CustomerData.Email\"")
        });
        then.status(200).json_body(json!({"id":"t1"}));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&[
            "glossary",
            "assign",
            "svc.db.s.orders",
            "--term",
            "CustomerData.Email",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    patch_hit.assert();
}

// ------- quality -------

#[test]
fn quality_list_hits_testcases() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/api/v1/dataQuality/testCases")
            .query_param("limit", "5")
            .query_param("entityLink", "<#E::table::svc.db.s.orders>");
        then.status(200).json_body(json!({
            "data": [{
                "fullyQualifiedName": "svc.db.s.orders.col_not_null",
                "name": "col_not_null",
                "description": "column is not null",
                "entityLink": "<#E::table::svc.db.s.orders>",
                "testSuite": {"name": "orders-suite"}
            }]
        }));
    });
    let home = TempHome::new(&server.base_url());
    let out = home
        .cmd(&[
            "quality",
            "list",
            "--table",
            "svc.db.s.orders",
            "--limit",
            "5",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    mock.assert();
    let s = stdout(&out);
    assert!(s.contains("col_not_null"));
}

// ------- completions -------

#[test]
fn completions_emits_non_empty_bash_script() {
    let home = TempHome::new("http://127.0.0.1:1");
    let out = home.cmd(&["completions", "bash"]).output().unwrap();
    assert!(out.status.success());
    let s = stdout(&out);
    assert!(s.contains("_omd"), "expected bash completion: {s}");
    assert!(s.contains("COMPREPLY"));
}

#[test]
fn completions_emits_zsh_script() {
    let home = TempHome::new("http://127.0.0.1:1");
    let out = home.cmd(&["completions", "zsh"]).output().unwrap();
    assert!(out.status.success());
    assert!(stdout(&out).contains("#compdef omd"));
}
