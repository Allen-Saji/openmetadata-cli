//! Helpers for working with OpenMetadata entities by FQN.
//!
//! Most smart commands need to: (1) discover an entity's type from its FQN,
//! (2) map that type to a REST collection path, and (3) fetch the entity
//! with a set of fields. These helpers centralize those steps so each
//! command doesn't re-implement them.

use crate::client::OmdClient;
use crate::error::{CliError, CliResult};
use crate::util::fqn;
use reqwest::Method;
use serde_json::Value;

/// Resolve an entity's type (`table`, `dashboard`, ...) by FQN via the
/// search API.
pub async fn resolve_type(client: &OmdClient, fqn: &str) -> CliResult<String> {
    let query = vec![
        ("q".to_string(), format!("fullyQualifiedName:\"{fqn}\"")),
        ("index".to_string(), "all".into()),
        ("size".to_string(), "1".into()),
    ];
    let v = client
        .json(Method::GET, "v1/search/query", &query, None)
        .await?;
    let hits = v
        .get("hits")
        .and_then(|h| h.get("hits"))
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();
    let first = hits.first().ok_or_else(|| CliError::NotFound(fqn.into()))?;
    first
        .get("_source")
        .and_then(|s| s.get("entityType"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| CliError::NotFound(fqn.into()))
}

/// REST collection path for an entity type (without leading slash).
pub fn endpoint_for_type(t: &str) -> String {
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
        "storedProcedure" => "v1/storedProcedures".into(),
        "searchIndex" => "v1/searchIndexes".into(),
        "apiCollection" => "v1/apiCollections".into(),
        "apiEndpoint" => "v1/apiEndpoints".into(),
        "dashboardDataModel" => "v1/dashboard/datamodels".into(),
        other => format!("v1/{other}s"),
    }
}

/// Percent-encode a string for use as a single path segment while preserving
/// dots, underscores, and dashes (common in OpenMetadata FQNs).
pub fn urlencode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Fetch an entity by FQN. If `entity_type` is None, resolves via search.
pub async fn fetch_by_fqn(
    client: &OmdClient,
    fqn: &str,
    entity_type: Option<&str>,
    fields: Option<&str>,
) -> CliResult<(String, serde_json::Value)> {
    let ty = match entity_type {
        Some(t) => t.to_string(),
        None => resolve_type(client, fqn).await?,
    };
    let path = format!("{}/name/{}", endpoint_for_type(&ty), urlencode_segment(fqn));
    let query = match fields {
        Some(f) => vec![("fields".to_string(), f.to_string())],
        None => vec![],
    };
    let v = client.json(Method::GET, &path, &query, None).await?;
    Ok((ty, v))
}

/// Extract the `id` of a fetched entity, if present.
pub fn entity_id(v: &serde_json::Value) -> Option<String> {
    v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string())
}

/// Result of resolving a user-supplied FQN: either a top-level entity, or a
/// column nested inside one (which OM models as part of the parent table, not
/// as a standalone entity).
pub enum FetchTarget {
    Entity {
        ty: String,
        id: String,
        value: Value,
    },
    Column {
        /// Parent table type (currently always `"table"`).
        ty: String,
        /// Parent table id.
        id: String,
        /// Parent table payload (with the requested fields).
        value: Value,
        /// Index of the column inside the table's `columns` array.
        column_index: usize,
        /// Column name (last segment of the original FQN).
        column_name: String,
    },
}

impl FetchTarget {
    pub fn id(&self) -> &str {
        match self {
            FetchTarget::Entity { id, .. } | FetchTarget::Column { id, .. } => id,
        }
    }

    pub fn ty(&self) -> &str {
        match self {
            FetchTarget::Entity { ty, .. } | FetchTarget::Column { ty, .. } => ty,
        }
    }
}

/// Resolve an FQN to a top-level entity or a column inside its parent table.
///
/// Routing rules:
/// 1. If `hint == Some("column")`, skip the entity fetch and treat the input
///    directly as a column FQN (the parent must be a table).
/// 2. Otherwise, try to fetch the FQN as a top-level entity first.
/// 3. If that returns NotFound, fall back to "treat as column": split off the
///    last segment, fetch the parent as a table, and find a matching column.
///    The parent fetch always asks for `columns` so the caller doesn't have
///    to thread that through.
pub async fn fetch_target(
    client: &OmdClient,
    fqn: &str,
    hint: Option<&str>,
    fields: Option<&str>,
) -> CliResult<FetchTarget> {
    if hint == Some("column") {
        return fetch_as_column(client, fqn, fields).await;
    }
    match fetch_by_fqn(client, fqn, hint, fields).await {
        Ok((ty, value)) => {
            let id = entity_id(&value)
                .ok_or_else(|| CliError::NotFound(format!("no id on entity {fqn}")))?;
            Ok(FetchTarget::Entity { ty, id, value })
        }
        Err(CliError::NotFound(_)) if hint.is_none() => fetch_as_column(client, fqn, fields).await,
        Err(e) => Err(e),
    }
}

async fn fetch_as_column(
    client: &OmdClient,
    fqn: &str,
    fields: Option<&str>,
) -> CliResult<FetchTarget> {
    let (parent_fqn, column_name) = fqn::split_last(fqn)
        .ok_or_else(|| CliError::NotFound(format!("cannot derive parent from FQN {fqn}")))?;

    // Always request columns; merge with caller-requested fields.
    let merged_fields = match fields {
        Some(f) if f.split(',').any(|s| s.trim() == "columns") => f.to_string(),
        Some(f) => format!("{f},columns"),
        None => "columns".to_string(),
    };

    let (ty, value) = fetch_by_fqn(client, &parent_fqn, Some("table"), Some(&merged_fields))
        .await
        .map_err(|e| match e {
            CliError::NotFound(_) => CliError::NotFound(fqn.to_string()),
            other => other,
        })?;
    let id = entity_id(&value)
        .ok_or_else(|| CliError::NotFound(format!("no id on parent table {parent_fqn}")))?;

    let columns = value
        .get("columns")
        .and_then(|c| c.as_array())
        .ok_or_else(|| CliError::NotFound(format!("parent table {parent_fqn} has no columns")))?;
    let column_index = columns
        .iter()
        .position(|c| c.get("name").and_then(|n| n.as_str()) == Some(column_name.as_str()))
        .ok_or_else(|| CliError::NotFound(format!("column {column_name} not in {parent_fqn}")))?;

    Ok(FetchTarget::Column {
        ty,
        id,
        value,
        column_index,
        column_name,
    })
}
