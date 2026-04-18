use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::CliResult;
use crate::output;
use crate::output::OutputCtx;
#[derive(clap::Args, Debug)]
pub struct SearchArgs {
    /// Search query
    pub query: String,
    /// Entity index (table, dashboard, pipeline, topic, etc.); defaults to all
    #[arg(long, default_value = "all")]
    pub index: String,
    /// Result limit
    #[arg(long, default_value_t = 10)]
    pub limit: u32,
    /// Result offset
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
}

pub async fn run(profile: &str, args: SearchArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let index_path = format!("{}_search_index", index_prefix(&args.index));
    let query = vec![
        ("q".to_string(), args.query.clone()),
        ("index".to_string(), index_path),
        ("from".to_string(), args.offset.to_string()),
        ("size".to_string(), args.limit.to_string()),
    ];

    let resp = client
        .json(reqwest::Method::GET, "v1/search/query", &query, None)
        .await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&resp);
        return Ok(());
    }

    let hits = resp
        .get("hits")
        .and_then(|h| h.get("hits"))
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();

    if hits.is_empty() {
        output::info(format!("no results for '{}'", args.query));
        return Ok(());
    }

    let rows: Vec<Vec<String>> = hits
        .iter()
        .map(|h| {
            let src = h.get("_source").cloned().unwrap_or_default();
            let name = s(&src, "fullyQualifiedName")
                .or_else(|| s(&src, "name"))
                .unwrap_or_default();
            let kind = s(&src, "entityType").unwrap_or_default();
            let svc = s(&src, "service")
                .or_else(|| {
                    src.get("service")
                        .and_then(|v| v.get("name"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            let desc = s(&src, "description").unwrap_or_default();
            vec![name, kind, svc, trim(&desc, 60)]
        })
        .collect();

    output::print_table(&["FQN", "TYPE", "SERVICE", "DESCRIPTION"], &rows);

    let total = resp
        .get("hits")
        .and_then(|h| h.get("total"))
        .and_then(|t| t.get("value"))
        .and_then(|v| v.as_u64())
        .unwrap_or(hits.len() as u64);
    output::info(format!("{} of {} results shown", hits.len(), total));
    Ok(())
}

fn s(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(String::from)
}

fn trim(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() > n {
        let head: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{head}…")
    } else {
        s
    }
}

fn index_prefix(idx: &str) -> String {
    match idx {
        "all" => "all".into(),
        "table" | "tables" => "table".into(),
        "dashboard" | "dashboards" => "dashboard".into(),
        "pipeline" | "pipelines" => "pipeline".into(),
        "topic" | "topics" => "topic".into(),
        "mlmodel" | "mlmodels" => "mlmodel".into(),
        "container" | "containers" => "container".into(),
        "glossary" | "glossaries" => "glossary".into(),
        "tag" | "tags" => "tag".into(),
        "user" | "users" => "user".into(),
        "team" | "teams" => "team".into(),
        other => other.to_string(),
    }
}
