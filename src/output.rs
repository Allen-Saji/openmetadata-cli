use crate::error::CliError;
use colored::Colorize;
use comfy_table::{ContentArrangement, Table};
use std::io::IsTerminal;

pub struct OutputCtx {
    pub json: bool,
}

pub fn is_stdout_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Returns true if we should use pretty output (TTY and not forced-json).
pub fn pretty(ctx: &OutputCtx) -> bool {
    !ctx.json && is_stdout_tty()
}

pub fn print_json(value: &serde_json::Value) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(_) => println!("{value}"),
    }
}

#[allow(dead_code)]
pub fn print_ndjson<I, V>(items: I)
where
    I: IntoIterator<Item = V>,
    V: serde::Serialize,
{
    for item in items {
        if let Ok(s) = serde_json::to_string(&item) {
            println!("{s}");
        }
    }
}

pub fn print_kv(pairs: &[(&str, String)]) {
    for (k, v) in pairs {
        println!("  {}: {}", k.bold(), v);
    }
}

pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut t = Table::new();
    t.set_content_arrangement(ContentArrangement::Dynamic);
    t.set_header(headers.iter().map(|h| h.to_string()));
    for row in rows {
        t.add_row(row.clone());
    }
    println!("{t}");
}

pub fn info(msg: impl AsRef<str>) {
    eprintln!("{} {}", "info".blue().bold(), msg.as_ref());
}

pub fn success(msg: impl AsRef<str>) {
    eprintln!("{} {}", "ok".green().bold(), msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    eprintln!("{} {}", "warn".yellow().bold(), msg.as_ref());
}

/// Render an arbitrary JSON response from the API.
///
/// TTY + not forced-JSON: best-effort pretty — table for arrays of objects,
/// kv list for single objects, else pretty JSON. Otherwise always JSON.
pub fn render_api_response(value: &serde_json::Value, ctx: &OutputCtx) {
    if !pretty(ctx) {
        print_json(value);
        return;
    }

    if let Some(arr) = as_list(value) {
        if arr.is_empty() {
            info("no results");
            return;
        }
        if let Some((headers, rows)) = table_from_objects(arr) {
            let hdr_refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();
            print_table(&hdr_refs, &rows);
            if let Some(paging) = paging_summary(value) {
                info(paging);
            }
            return;
        }
    }

    if let Some(obj) = value.as_object() {
        if has_simple_leaf(obj) {
            let pairs: Vec<(&str, String)> = obj
                .iter()
                .filter_map(|(k, v)| simple_leaf_value(v).map(|s| (k.as_str(), s)))
                .collect();
            if !pairs.is_empty() {
                print_kv(&pairs);
                return;
            }
        }
    }

    print_json(value);
}

fn as_list(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    if let Some(a) = value.as_array() {
        return Some(a);
    }
    value.get("data").and_then(|v| v.as_array())
}

fn table_from_objects(arr: &[serde_json::Value]) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    if arr.iter().any(|v| !v.is_object()) {
        return None;
    }
    const CANDIDATES: &[&str] = &[
        "id",
        "name",
        "fullyQualifiedName",
        "displayName",
        "entityType",
        "service",
        "description",
        "updatedAt",
    ];
    let mut headers: Vec<String> = Vec::new();
    for c in CANDIDATES {
        if arr.iter().any(|v| v.get(c).is_some()) {
            headers.push((*c).into());
        }
        if headers.len() >= 5 {
            break;
        }
    }
    if headers.is_empty() {
        if let Some(first) = arr.first().and_then(|v| v.as_object()) {
            headers = first.keys().take(5).cloned().collect();
        }
    }
    if headers.is_empty() {
        return None;
    }
    let rows: Vec<Vec<String>> = arr
        .iter()
        .map(|v| {
            headers
                .iter()
                .map(|h| {
                    v.get(h)
                        .map(|x| simple_leaf_value(x).unwrap_or_else(|| summarize_nested(x)))
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect();
    Some((headers, rows))
}

fn has_simple_leaf(obj: &serde_json::Map<String, serde_json::Value>) -> bool {
    obj.values().any(|v| simple_leaf_value(v).is_some())
}

fn simple_leaf_value(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => None,
        _ => None,
    }
}

fn summarize_nested(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Array(a) => format!("[{} items]", a.len()),
        serde_json::Value::Object(o) => o
            .get("name")
            .or_else(|| o.get("fullyQualifiedName"))
            .and_then(|x| x.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("{{{} fields}}", o.len())),
        _ => String::new(),
    }
}

fn paging_summary(value: &serde_json::Value) -> Option<String> {
    let paging = value.get("paging")?;
    let total = paging.get("total").and_then(|v| v.as_u64());
    let after = paging.get("after").and_then(|v| v.as_str());
    let before = paging.get("before").and_then(|v| v.as_str());
    let mut parts = Vec::new();
    if let Some(t) = total {
        parts.push(format!("total {t}"));
    }
    if let Some(a) = after {
        parts.push(format!("next cursor: {a}"));
    }
    if let Some(b) = before {
        parts.push(format!("prev cursor: {b}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("  "))
    }
}

pub fn render_error(err: &CliError) {
    // If stderr is not a TTY, emit structured JSON for downstream parsing.
    if !std::io::stderr().is_terminal() {
        let j = serde_json::json!({
            "error": {
                "kind": err.kind(),
                "message": err.to_string(),
                "exit_code": err.exit_code(),
            }
        });
        let _ = serde_json::to_writer(std::io::stderr(), &j);
        eprintln!();
    } else {
        eprintln!("{} {}", "error".red().bold(), err);
    }
}
