//! `omd tag <fqn> --add ... --remove ...` — mutate the tags array on an entity
//! or on a column nested inside its parent table.

use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output::{self, OutputCtx};
use crate::util::entity::{self, FetchTarget};
use serde_json::{json, Value};

#[derive(clap::Args, Debug)]
pub struct TagArgs {
    /// Fully-qualified name of the entity.
    pub fqn: String,

    /// Entity type hint (resolved via search if omitted). Use `column` to
    /// force column routing when the FQN is ambiguous.
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

    let target =
        entity::fetch_target(&client, &args.fqn, args.r#type.as_deref(), Some("tags")).await?;

    let patch = build_tag_patch(&target, &args.add, &args.remove);

    if args.dry_run {
        output::print_json(&patch);
        return Ok(());
    }

    let path = format!("{}/{}", entity::endpoint_for_type(target.ty()), target.id());
    let updated = client.json_patch(&path, &patch).await?;

    if ctx.json || !output::pretty(ctx) {
        output::print_json(&updated);
    } else {
        let label = match &target {
            FetchTarget::Entity { ty, .. } => format!("{ty} {}", args.fqn),
            FetchTarget::Column { column_name, .. } => format!("column {}", column_name),
        };
        output::success(format!(
            "tags updated on {label} (+{}, -{})",
            args.add.len(),
            args.remove.len()
        ));
    }
    Ok(())
}

fn build_tag_patch(target: &FetchTarget, add: &[String], remove: &[String]) -> Value {
    let (current_tags_obj, patch_path) = match target {
        FetchTarget::Entity { value, .. } => (value.get("tags"), "/tags".to_string()),
        FetchTarget::Column {
            value,
            column_index,
            ..
        } => {
            let col = value
                .get("columns")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.get(*column_index));
            (
                col.and_then(|c| c.get("tags")),
                format!("/columns/{column_index}/tags"),
            )
        }
    };
    let new_tags = mutate_tags(current_tags_obj, add, remove);
    let op = if current_tags_obj.is_some() {
        "replace"
    } else {
        "add"
    };
    json!([{ "op": op, "path": patch_path, "value": new_tags }])
}

fn mutate_tags(current: Option<&Value>, add: &[String], remove: &[String]) -> Vec<Value> {
    let mut tags: Vec<Value> = current
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
