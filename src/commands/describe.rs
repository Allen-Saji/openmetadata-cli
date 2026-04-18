use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output;
use crate::output::OutputCtx;
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

    let entity_type = match args.r#type {
        Some(t) => t,
        None => resolve_entity_type(&client, &args.fqn).await?,
    };

    let path = endpoint_for_type(&entity_type);
    let fqn_enc = urlencode(&args.fqn);
    let url_path = format!("{path}/name/{fqn_enc}");
    let query = vec![("fields".to_string(), args.fields.clone())];

    let v = client
        .json(reqwest::Method::GET, &url_path, &query, None)
        .await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&v);
        return Ok(());
    }

    pretty_entity(&entity_type, &v);
    Ok(())
}

async fn resolve_entity_type(client: &OmdClient, fqn: &str) -> CliResult<String> {
    let query = vec![
        ("q".to_string(), format!("fullyQualifiedName:\"{fqn}\"")),
        ("index".to_string(), "all".into()),
        ("size".to_string(), "1".into()),
    ];
    let v = client
        .json(reqwest::Method::GET, "v1/search/query", &query, None)
        .await?;
    let hits = v
        .get("hits")
        .and_then(|h| h.get("hits"))
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();
    let first = hits.first().ok_or_else(|| CliError::NotFound(fqn.into()))?;
    let t = first
        .get("_source")
        .and_then(|s| s.get("entityType"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| CliError::NotFound(fqn.into()))?;
    Ok(t.to_string())
}

fn endpoint_for_type(t: &str) -> String {
    match t {
        "table" => "v1/tables".into(),
        "dashboard" => "v1/dashboards".into(),
        "pipeline" => "v1/pipelines".into(),
        "topic" => "v1/topics".into(),
        "mlmodel" => "v1/mlmodels".into(),
        "container" => "v1/containers".into(),
        "database" => "v1/databases".into(),
        "databaseSchema" => "v1/databaseSchemas".into(),
        "glossary" => "v1/glossaries".into(),
        "glossaryTerm" => "v1/glossaryTerms".into(),
        "tag" => "v1/tags".into(),
        "user" => "v1/users".into(),
        "team" => "v1/teams".into(),
        other => format!("v1/{other}s"),
    }
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

fn urlencode(s: &str) -> String {
    // Preserve dots/underscores/dashes in FQNs; percent-encode the rest.
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
