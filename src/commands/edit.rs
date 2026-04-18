//! `omd edit <fqn>` — update entity fields via JSON patch.

use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output::{self, OutputCtx};
use crate::util::entity;
use reqwest::Method;
use serde_json::{json, Value};

#[derive(clap::Args, Debug)]
pub struct EditArgs {
    /// Fully-qualified name of the entity.
    pub fqn: String,

    /// Entity type hint (resolved via search if omitted).
    #[arg(long)]
    pub r#type: Option<String>,

    /// New description. Inline text, or `@path/to/file.md` for a file.
    #[arg(long)]
    pub description: Option<String>,

    /// New display name.
    #[arg(long = "display-name")]
    pub display_name: Option<String>,

    /// Owner FQN (user or team). Looked up via search to resolve the ID.
    #[arg(long)]
    pub owner: Option<String>,

    /// Tier classification FQN, e.g. `Tier.Tier1`.
    #[arg(long)]
    pub tier: Option<String>,

    /// Print the computed JSON patch without sending it.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(profile: &str, args: EditArgs, ctx: &OutputCtx) -> CliResult<()> {
    if args.description.is_none()
        && args.display_name.is_none()
        && args.owner.is_none()
        && args.tier.is_none()
    {
        return Err(CliError::InvalidInput(
            "nothing to update: pass one or more of --description, --display-name, --owner, --tier"
                .into(),
        ));
    }

    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let (entity_type, current) = entity::fetch_by_fqn(
        &client,
        &args.fqn,
        args.r#type.as_deref(),
        Some("owners,tags"),
    )
    .await?;

    let id = entity::entity_id(&current)
        .ok_or_else(|| CliError::NotFound(format!("no id on entity {}", args.fqn)))?;

    let mut ops: Vec<Value> = Vec::new();

    if let Some(d) = args.description {
        let text = read_inline_or_file(&d)?;
        ops.push(upsert(&current, "/description", json!(text)));
    }
    if let Some(n) = args.display_name {
        ops.push(upsert(&current, "/displayName", json!(n)));
    }
    if let Some(owner_fqn) = args.owner {
        let owner_ref = resolve_owner(&client, &owner_fqn).await?;
        ops.push(upsert(&current, "/owners", json!([owner_ref])));
    }
    if let Some(tier_fqn) = args.tier {
        let new_tags = upsert_tier(&current, &tier_fqn);
        ops.push(upsert(&current, "/tags", json!(new_tags)));
    }

    let patch = Value::Array(ops);
    if args.dry_run {
        output::print_json(&patch);
        return Ok(());
    }

    let path = format!("{}/{}", entity::endpoint_for_type(&entity_type), id);
    let updated = client.json_patch(&path, &patch).await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&updated);
    } else {
        output::success(format!("updated {} {}", entity_type, args.fqn));
        output::render_api_response(&updated, ctx);
    }
    Ok(())
}

fn upsert(current: &Value, path: &str, value: Value) -> Value {
    let key = path.trim_start_matches('/');
    let op = if current.get(key).is_some() {
        "replace"
    } else {
        "add"
    };
    json!({ "op": op, "path": path, "value": value })
}

fn read_inline_or_file(raw: &str) -> CliResult<String> {
    if let Some(p) = raw.strip_prefix('@') {
        Ok(std::fs::read_to_string(p)?)
    } else {
        Ok(raw.to_string())
    }
}

async fn resolve_owner(client: &OmdClient, owner_fqn: &str) -> CliResult<Value> {
    let query = vec![
        (
            "q".to_string(),
            format!("fullyQualifiedName:\"{owner_fqn}\""),
        ),
        ("index".to_string(), "all".into()),
        ("size".to_string(), "1".into()),
    ];
    let v = client
        .json(Method::GET, "v1/search/query", &query, None)
        .await?;
    let first = v
        .get("hits")
        .and_then(|h| h.get("hits"))
        .and_then(|h| h.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| CliError::NotFound(format!("owner `{owner_fqn}`")))?;
    let src = first
        .get("_source")
        .ok_or_else(|| CliError::NotFound(format!("owner `{owner_fqn}`")))?;
    let id = src
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CliError::NotFound(format!("owner `{owner_fqn}` missing id")))?;
    let entity_type = src
        .get("entityType")
        .and_then(|v| v.as_str())
        .unwrap_or("user");
    Ok(json!({
        "id": id,
        "type": entity_type,
        "fullyQualifiedName": owner_fqn,
    }))
}

fn upsert_tier(current: &Value, tier_fqn: &str) -> Vec<Value> {
    let mut tags: Vec<Value> = current
        .get("tags")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| !is_tier_tag(t))
        .collect();
    tags.push(json!({
        "tagFQN": tier_fqn,
        "source": "Classification",
        "labelType": "Manual",
        "state": "Confirmed",
    }));
    tags
}

fn is_tier_tag(t: &Value) -> bool {
    t.get("tagFQN")
        .and_then(|v| v.as_str())
        .map(|s| s.starts_with("Tier."))
        .unwrap_or(false)
}
