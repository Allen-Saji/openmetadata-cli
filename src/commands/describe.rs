use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::CliResult;
use crate::output;
use crate::output::OutputCtx;
use crate::util::entity;

#[derive(clap::Args, Debug)]
pub struct DescribeArgs {
    /// Fully-qualified name of the entity (e.g. service.database.schema.table)
    pub fqn: String,
    /// Entity type hint (table, dashboard, pipeline, topic, user, ...).
    /// If omitted, resolves automatically via the search API.
    #[arg(long)]
    pub r#type: Option<String>,
    /// Additional fields to include (comma-separated, e.g. "owners,tags,columns")
    #[arg(long, default_value = "owners,tags,followers")]
    pub fields: String,
}

pub async fn run(profile: &str, args: DescribeArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let (entity_type, v) = entity::fetch_by_fqn(
        &client,
        &args.fqn,
        args.r#type.as_deref(),
        Some(&args.fields),
    )
    .await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&v);
        return Ok(());
    }

    pretty_entity(&entity_type, &v);
    Ok(())
}

fn pretty_entity(t: &str, v: &serde_json::Value) {
    let name = v
        .get("fullyQualifiedName")
        .or_else(|| v.get("name"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let desc = v.get("description").and_then(|s| s.as_str()).unwrap_or("");
    let owners = v
        .get("owners")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|u| {
                    u.get("displayName")
                        .and_then(|s| s.as_str())
                        .or_else(|| u.get("name").and_then(|s| s.as_str()))
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let tags = v
        .get("tags")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("tagFQN").and_then(|s| s.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let pairs: Vec<(&str, String)> = vec![
        ("type", t.to_string()),
        ("fqn", name.to_string()),
        ("description", desc.to_string()),
        ("owners", owners),
        ("tags", tags),
    ];
    output::print_kv(&pairs);

    // Table-specific: columns
    if t == "table" {
        if let Some(cols) = v.get("columns").and_then(|c| c.as_array()) {
            let rows: Vec<Vec<String>> = cols
                .iter()
                .map(|c| {
                    let n = c
                        .get("name")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let dt = c
                        .get("dataType")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let d = c
                        .get("description")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    vec![n, dt, trunc(&d, 50)]
                })
                .collect();
            if !rows.is_empty() {
                println!();
                println!("columns:");
                output::print_table(&["NAME", "TYPE", "DESCRIPTION"], &rows);
            }
        }
    }
}

fn trunc(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() > n {
        let head: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{head}…")
    } else {
        s
    }
}
