//! Helpers for working with OpenMetadata entities by FQN.
//!
//! Most smart commands need to: (1) discover an entity's type from its FQN,
//! (2) map that type to a REST collection path, and (3) fetch the entity
//! with a set of fields. These helpers centralize those steps so each
//! command doesn't re-implement them.

use crate::client::OmdClient;
use crate::error::{CliError, CliResult};
use reqwest::Method;

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
