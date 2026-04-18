//! `omd quality` — data quality test case browsing.

use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::CliResult;
use crate::output::{self, OutputCtx};
use crate::util::entity;
use reqwest::Method;

#[derive(clap::Subcommand, Debug)]
pub enum Action {
    /// List test cases, optionally filtered to one table.
    List(ListArgs),
    /// Show recent results for a single test case.
    Results(ResultsArgs),
    /// Show the latest result for each test case on a table.
    Latest(LatestArgs),
}

#[derive(clap::Args, Debug)]
pub struct ListArgs {
    /// Scope to a table FQN.
    #[arg(long)]
    pub table: Option<String>,
    /// Max results.
    #[arg(long, default_value_t = 25)]
    pub limit: u32,
    /// Additional fields to include.
    #[arg(long, default_value = "testSuite,entityLink")]
    pub fields: String,
}

#[derive(clap::Args, Debug)]
pub struct ResultsArgs {
    /// Fully-qualified name of the test case.
    pub fqn: String,
    /// Max results to show.
    #[arg(long, default_value_t = 10)]
    pub limit: u32,
}

#[derive(clap::Args, Debug)]
pub struct LatestArgs {
    /// Table FQN to pull latest results for.
    pub table: String,
}

pub async fn run(profile: &str, action: Action, ctx: &OutputCtx) -> CliResult<()> {
    match action {
        Action::List(args) => list(profile, args, ctx).await,
        Action::Results(args) => results(profile, args, ctx).await,
        Action::Latest(args) => latest(profile, args, ctx).await,
    }
}

async fn list(profile: &str, args: ListArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let mut query: Vec<(String, String)> = vec![
        ("limit".into(), args.limit.to_string()),
        ("fields".into(), args.fields.clone()),
    ];
    if let Some(table) = args.table {
        query.push(("entityLink".into(), entity_link_for_table(&table)));
    }
    let v = client
        .json(Method::GET, "v1/dataQuality/testCases", &query, None)
        .await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&v);
        return Ok(());
    }

    render_test_cases(&v);
    Ok(())
}

async fn results(profile: &str, args: ResultsArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let path = format!(
        "v1/dataQuality/testCases/testCaseResults/{}",
        entity::urlencode_segment(&args.fqn)
    );
    let query: Vec<(String, String)> = vec![("limit".into(), args.limit.to_string())];
    let v = client.json(Method::GET, &path, &query, None).await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&v);
        return Ok(());
    }
    render_results(&v);
    Ok(())
}

async fn latest(profile: &str, args: LatestArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let query: Vec<(String, String)> = vec![
        (
            "q".into(),
            format!("entityLink:\"{}\"", entity_link_for_table(&args.table)),
        ),
        ("size".into(), "100".into()),
    ];
    let v = client
        .json(
            Method::GET,
            "v1/dataQuality/testCases/testCaseResults/search/latest",
            &query,
            None,
        )
        .await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&v);
        return Ok(());
    }
    render_results(&v);
    Ok(())
}

fn entity_link_for_table(fqn: &str) -> String {
    format!("<#E::table::{fqn}>")
}

fn render_test_cases(v: &serde_json::Value) {
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    if data.is_empty() {
        output::info("no test cases");
        return;
    }
    let rows: Vec<Vec<String>> = data
        .iter()
        .map(|tc| {
            let name = tc
                .get("fullyQualifiedName")
                .or_else(|| tc.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let suite = tc
                .get("testSuite")
                .and_then(|s| s.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let link = tc
                .get("entityLink")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let desc = tc
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            vec![name, suite, link, truncate(&desc, 40)]
        })
        .collect();
    output::print_table(&["FQN", "SUITE", "ENTITY_LINK", "DESCRIPTION"], &rows);
}

fn render_results(v: &serde_json::Value) {
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    if data.is_empty() {
        output::info("no results");
        return;
    }
    let rows: Vec<Vec<String>> = data
        .iter()
        .map(|r| {
            let status = r
                .get("testCaseStatus")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let ts = r
                .get("timestamp")
                .and_then(|v| v.as_i64())
                .map(format_millis)
                .unwrap_or_default();
            let result = r
                .get("result")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = r
                .get("testCaseFQN")
                .or_else(|| r.get("testCase"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            vec![ts, status, name, truncate(&result, 60)]
        })
        .collect();
    output::print_table(&["WHEN", "STATUS", "TEST_CASE", "RESULT"], &rows);
}

fn format_millis(ms: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};
    let t = UNIX_EPOCH + Duration::from_millis(ms as u64);
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    // Simple yyyy-mm-dd HH:MM:SS UTC without chrono dep.
    let (year, month, day, hour, min, sec) = epoch_to_ymdhms(secs as i64);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}Z")
}

fn epoch_to_ymdhms(epoch: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = epoch.div_euclid(86_400);
    let secs_of_day = epoch.rem_euclid(86_400) as u32;
    let hour = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    let sec = secs_of_day % 60;
    let (year, month, day) = days_to_ymd(days);
    (year, month, day, hour, min, sec)
}

fn days_to_ymd(mut days: i64) -> (i32, u32, u32) {
    // Shift so day 0 = March 1, 2000 (makes leap-year math simple).
    days += 719_468; // days from 0000-03-01 to 1970-01-01
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() > n {
        let head: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{head}…")
    } else {
        s
    }
}
