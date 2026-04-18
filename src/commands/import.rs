//! `omd import <type> <fqn> <file.csv>` — upload CSV to update entity metadata.
//!
//! By default the server runs a dry-run validation. Pass `--apply` to commit.

use crate::client::{read_json, OmdClient};
use crate::config::ResolvedConfig;
use crate::error::CliResult;
use crate::output::{self, OutputCtx};
use crate::util::{csv, entity};
use colored::Colorize;
use indicatif::ProgressBar;
use reqwest::Method;
use std::path::PathBuf;
use std::time::Duration;

#[derive(clap::Args, Debug)]
pub struct ImportArgs {
    /// Entity type (same set as `omd export`).
    pub r#type: String,

    /// Fully-qualified name of the target entity.
    pub fqn: String,

    /// Path to a local CSV file, or `-` to read from stdin.
    pub file: PathBuf,

    /// Actually apply changes. Without this flag the server runs in dry-run
    /// mode and returns a report of what would change.
    #[arg(long)]
    pub apply: bool,
}

pub async fn run(profile: &str, args: ImportArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let collection = csv::collection_for(&args.r#type)?;
    let path = format!(
        "{collection}/name/{}/import",
        entity::urlencode_segment(&args.fqn)
    );

    let body = read_csv(&args.file)?;
    let query = vec![("dryRun".to_string(), (!args.apply).to_string())];

    let spinner = if output::pretty(ctx) {
        let p = ProgressBar::new_spinner();
        p.set_message(format!(
            "{} {} {} ({} bytes)",
            if args.apply {
                "importing"
            } else {
                "validating"
            },
            args.r#type,
            args.fqn,
            body.len()
        ));
        p.enable_steady_tick(Duration::from_millis(100));
        Some(p)
    } else {
        None
    };

    let resp = client
        .request_text(Method::PUT, &path, &query, body)
        .await?;
    let result = read_json(resp).await?;

    if let Some(p) = spinner {
        p.finish_and_clear();
    }

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&result);
        if let Some(status) = result.get("status").and_then(|v| v.as_str()) {
            if matches!(status, "failure" | "aborted") {
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    render_result(&args, &result);
    Ok(())
}

fn read_csv(p: &std::path::Path) -> CliResult<String> {
    if p.as_os_str() == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        Ok(std::fs::read_to_string(p)?)
    }
}

fn render_result(args: &ImportArgs, v: &serde_json::Value) {
    let status = v.get("status").and_then(|v| v.as_str()).unwrap_or("?");
    let dry = v
        .get("dryRun")
        .and_then(|v| v.as_bool())
        .unwrap_or(!args.apply);
    let processed = v
        .get("numberOfRowsProcessed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let passed = v
        .get("numberOfRowsPassed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let failed = v
        .get("numberOfRowsFailed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let colored_status = match status {
        "success" => status.green().bold().to_string(),
        "partialSuccess" => status.yellow().bold().to_string(),
        "failure" | "aborted" => status.red().bold().to_string(),
        _ => status.bold().to_string(),
    };
    let mode = if dry { "dry-run" } else { "applied" };

    output::print_kv(&[
        ("type", args.r#type.clone()),
        ("fqn", args.fqn.clone()),
        ("mode", mode.into()),
        ("status", colored_status),
        ("processed", processed.to_string()),
        ("passed", passed.to_string()),
        ("failed", failed.to_string()),
    ]);

    if let Some(reason) = v.get("abortReason").and_then(|v| v.as_str()) {
        if !reason.is_empty() {
            output::warn(format!("abort reason: {reason}"));
        }
    }

    if let Some(rc) = v.get("importResultsCsv").and_then(|v| v.as_str()) {
        if !rc.trim().is_empty() && failed > 0 {
            eprintln!();
            eprintln!("{}", "failed rows:".bold());
            eprintln!("{rc}");
        }
    }

    if matches!(status, "failure" | "aborted") {
        std::process::exit(1);
    }
    if dry && processed > 0 && failed == 0 {
        output::info("dry-run ok — pass --apply to commit");
    }
}
