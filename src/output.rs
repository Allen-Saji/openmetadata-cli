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
