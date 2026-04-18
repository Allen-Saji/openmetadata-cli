//! `omd tag <fqn> --add ... --remove ...` — mutate the tags array on an entity.

use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output::{self, OutputCtx};
use crate::util::entity;
use serde_json::{json, Value};

#[derive(clap::Args, Debug)]
pub struct TagArgs {
    /// Fully-qualified name of the entity.
    pub fqn: String,

    /// Entity type hint (resolved via search if omitted).
    #[arg(long)]
    pub r#type: Option<String>,

    /// Tag FQN to add (repeatable). Example: `PII.Sensitive`.
    #[arg(long = "add")]
    pub add: Vec<String>,

    /// Tag FQN to remove (repeatable).
    #[arg(long = "remove")]
    pub remove: Vec<String>,

    /// Print the computed JSON patch without sending it.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(profile: &str, args: TagArgs, ctx: &OutputCtx) -> CliResult<()> {
    if args.add.is_empty() && args.remove.is_empty() {
        return Err(CliError::InvalidInput(
            "nothing to do: pass --add and/or --remove".into(),
        ));
    }

    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let (entity_type, current) =
        entity::fetch_by_fqn(&client, &args.fqn, args.r#type.as_deref(), Some("tags")).await?;
    let id = entity::entity_id(&current)
        .ok_or_else(|| CliError::NotFound(format!("no id on entity {}", args.fqn)))?;

    let new_tags = mutate_tags(&current, &args.add, &args.remove);
    let op = if current.get("tags").is_some() {
        "replace"
    } else {
        "add"
    };
    let patch = json!([{ "op": op, "path": "/tags", "value": new_tags }]);

    if args.dry_run {
        output::print_json(&patch);
        return Ok(());
    }

    let path = format!("{}/{}", entity::endpoint_for_type(&entity_type), id);
    let updated = client.json_patch(&path, &patch).await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&updated);
    } else {
        output::success(format!(
            "tags updated on {} {} (+{}, -{})",
            entity_type,
            args.fqn,
            args.add.len(),
            args.remove.len()
        ));
    }
    Ok(())
}

fn mutate_tags(current: &Value, add: &[String], remove: &[String]) -> Vec<Value> {
    let mut tags: Vec<Value> = current
        .get("tags")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    tags.retain(|t| {
        let fqn = t.get("tagFQN").and_then(|v| v.as_str()).unwrap_or("");
        !remove.iter().any(|r| r == fqn)
    });
    for new_fqn in add {
        if tags.iter().any(|t| {
            t.get("tagFQN")
                .and_then(|v| v.as_str())
                .map(|s| s == new_fqn)
                .unwrap_or(false)
        }) {
            continue;
        }
        tags.push(json!({
            "tagFQN": new_fqn,
            "source": "Classification",
            "labelType": "Manual",
            "state": "Confirmed",
        }));
    }
    tags
}
