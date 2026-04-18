//! In-memory index of an OpenAPI 3 spec.
//!
//! Groups operations by tag and normalizes action names so they can be
//! exposed as `omd <group> <action>` commands at runtime.

use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamKind {
    String,
    Integer,
    Number,
    Boolean,
    Array,
}

impl ParamKind {
    fn from_schema(schema: Option<&Value>) -> Self {
        let t = schema.and_then(|s| s.get("type")).and_then(|s| s.as_str());
        match t {
            Some("integer") => ParamKind::Integer,
            Some("number") => ParamKind::Number,
            Some("boolean") => ParamKind::Boolean,
            Some("array") => ParamKind::Array,
            _ => ParamKind::String,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub required: bool,
    pub kind: ParamKind,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Operation {
    #[allow(dead_code)] // surfaced for diagnostics + MCP tool-id mapping later
    pub op_id: String,
    pub action: String,
    pub method: String,
    pub path: String,
    pub path_params: Vec<Param>,
    pub query_params: Vec<Param>,
    pub has_body: bool,
    pub body_required: bool,
    pub summary: Option<String>,
}

#[derive(Debug, Default)]
pub struct Index {
    pub groups: BTreeMap<String, Vec<Operation>>,
    pub tag_original: BTreeMap<String, String>,
}

impl Index {
    /// Build an index from a parsed OpenAPI JSON value.
    pub fn from_spec(spec: &Value) -> Self {
        let mut idx = Index::default();
        let paths = match spec.get("paths").and_then(|p| p.as_object()) {
            Some(p) => p,
            None => return idx,
        };

        for (path, methods) in paths {
            let obj = match methods.as_object() {
                Some(o) => o,
                None => continue,
            };
            let path_level_params = extract_params(obj.get("parameters"));

            for (method, op) in obj {
                let m = method.to_lowercase();
                if !matches!(m.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                    continue;
                }
                let tag_raw = op
                    .get("tags")
                    .and_then(|t| t.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .unwrap_or("misc")
                    .to_string();

                let op_id = op
                    .get("operationId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if op_id.is_empty() {
                    continue;
                }

                let mut params = path_level_params.clone();
                params.extend(extract_params(op.get("parameters")));

                let path_params = params
                    .iter()
                    .filter(|p| p.in_ == "path")
                    .map(|p| p.to_param())
                    .collect::<Vec<_>>();
                let query_params = params
                    .iter()
                    .filter(|p| p.in_ == "query")
                    .map(|p| p.to_param())
                    .collect::<Vec<_>>();

                let body = op.get("requestBody");
                let has_body = body.is_some();
                let body_required = body
                    .and_then(|b| b.get("required"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let summary = op
                    .get("summary")
                    .or_else(|| op.get("description"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.lines().next().unwrap_or("").trim().to_string())
                    .filter(|s| !s.is_empty());

                let action = action_name(&op_id, &tag_raw);
                let group = kebab(&tag_raw);

                let operation = Operation {
                    op_id,
                    action,
                    method: m.to_uppercase(),
                    path: path.clone(),
                    path_params,
                    query_params,
                    has_body,
                    body_required,
                    summary,
                };

                idx.tag_original
                    .entry(group.clone())
                    .or_insert(tag_raw.clone());
                idx.groups.entry(group).or_default().push(operation);
            }
        }

        for ops in idx.groups.values_mut() {
            disambiguate_actions(ops);
        }

        idx
    }

    pub fn groups(&self) -> Vec<&str> {
        self.groups.keys().map(|s| s.as_str()).collect()
    }

    pub fn get(&self, group: &str) -> Option<&[Operation]> {
        self.groups.get(group).map(|v| v.as_slice())
    }
}

#[derive(Debug, Clone)]
struct RawParam {
    name: String,
    in_: String,
    required: bool,
    kind: ParamKind,
    description: Option<String>,
}

impl RawParam {
    fn to_param(&self) -> Param {
        Param {
            name: self.name.clone(),
            required: self.required,
            kind: self.kind.clone(),
            description: self.description.clone(),
        }
    }
}

fn extract_params(v: Option<&Value>) -> Vec<RawParam> {
    let arr = match v.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return vec![],
    };
    arr.iter()
        .filter_map(|p| {
            let name = p.get("name").and_then(|v| v.as_str())?.to_string();
            let in_ = p
                .get("in")
                .and_then(|v| v.as_str())
                .unwrap_or("query")
                .to_string();
            let required = p
                .get("required")
                .and_then(|v| v.as_bool())
                .unwrap_or(in_ == "path");
            let kind = ParamKind::from_schema(p.get("schema"));
            let description = p
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.lines().next().unwrap_or("").trim().to_string())
                .filter(|s| !s.is_empty());
            Some(RawParam {
                name,
                in_,
                required,
                kind,
                description,
            })
        })
        .collect()
}

/// Convert camelCase / snake_case / "Space Cased" → `kebab-case`.
pub fn kebab(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 4);
    for i in 0..chars.len() {
        let c = chars[i];
        if c == '_' || c == '-' || c.is_whitespace() {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            continue;
        }
        if !c.is_ascii_alphanumeric() {
            continue;
        }
        if c.is_ascii_uppercase() {
            let prev_lower_or_digit = chars
                .get(i.wrapping_sub(1))
                .copied()
                .map(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
                .unwrap_or(false);
            let next_lower = chars
                .get(i + 1)
                .copied()
                .map(|n| n.is_ascii_lowercase())
                .unwrap_or(false);
            if !out.is_empty() && !out.ends_with('-') && (prev_lower_or_digit || next_lower) {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn depluralize(s: &str) -> String {
    if s.ends_with("ies") && s.len() > 3 {
        return format!("{}y", &s[..s.len() - 3]);
    }
    if s.ends_with("sses") {
        return s[..s.len() - 2].to_string();
    }
    if s.ends_with('s') && !s.ends_with("ss") && !s.ends_with("us") && !s.ends_with("is") {
        return s[..s.len() - 1].to_string();
    }
    s.to_string()
}

fn strip_tag_words(op_kebab: &str, tag_kebab: &str) -> Option<String> {
    if tag_kebab.is_empty() {
        return None;
    }
    let tag_words: Vec<&str> = tag_kebab.split('-').filter(|w| !w.is_empty()).collect();
    let op_words: Vec<&str> = op_kebab.split('-').filter(|w| !w.is_empty()).collect();
    if tag_words.is_empty() || op_words.len() < tag_words.len() {
        return None;
    }
    for start in 0..=op_words.len().saturating_sub(tag_words.len()) {
        if op_words[start..start + tag_words.len()] == tag_words[..] {
            let mut kept: Vec<&str> = Vec::with_capacity(op_words.len());
            kept.extend_from_slice(&op_words[..start]);
            kept.extend_from_slice(&op_words[start + tag_words.len()..]);
            return Some(kept.join("-"));
        }
    }
    None
}

/// Derive an action name for an operationId given its tag.
///
/// Strips the tag (plural + singular form) from a kebab-cased operationId.
/// Falls back to the full kebab-cased operationId if stripping yields nothing.
pub fn action_name(op_id: &str, tag: &str) -> String {
    let op_kebab = kebab(op_id);
    let tag_kebab = kebab(tag);
    let stripped = strip_tag_words(&op_kebab, &tag_kebab)
        .or_else(|| strip_tag_words(&op_kebab, &depluralize(&tag_kebab)));
    match stripped {
        Some(s) if !s.is_empty() => s,
        _ => op_kebab,
    }
}

fn disambiguate_actions(ops: &mut [Operation]) {
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for op in ops.iter() {
        *counts.entry(op.action.clone()).or_insert(0) += 1;
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for op in ops.iter_mut() {
        if counts.get(&op.action).copied().unwrap_or(0) <= 1 {
            continue;
        }
        let idx = seen.entry(op.action.clone()).or_insert(0);
        if *idx > 0 {
            op.action = format!("{}-{}", op.action, idx);
        }
        *idx += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_basic() {
        assert_eq!(kebab("listTables"), "list-tables");
        assert_eq!(kebab("getTableByID"), "get-table-by-id");
        assert_eq!(kebab("AI Applications"), "ai-applications");
        assert_eq!(kebab("getAIApplicationByID"), "get-ai-application-by-id");
        assert_eq!(kebab("patchAIApplication_1"), "patch-ai-application-1");
        assert_eq!(kebab("HTTP_Server"), "http-server");
    }

    #[test]
    fn action_strips_tag() {
        assert_eq!(action_name("listTables", "Tables"), "list");
        assert_eq!(action_name("getTableByID", "Tables"), "get-by-id");
        assert_eq!(
            action_name("createAIApplication", "AI Applications"),
            "create"
        );
        assert_eq!(
            action_name("getAIApplicationByFQN", "AI Applications"),
            "get-by-fqn"
        );
        assert_eq!(
            action_name("patchAIApplication_1", "AI Applications"),
            "patch-1"
        );
        assert_eq!(
            action_name("addFollower", "AI Applications"),
            "add-follower"
        );
    }

    #[test]
    fn builds_small_index() {
        let spec = serde_json::json!({
            "paths": {
                "/v1/tables": {
                    "get": {
                        "operationId": "listTables",
                        "tags": ["Tables"],
                        "parameters": [
                            { "name": "limit", "in": "query", "schema": { "type": "integer" } }
                        ]
                    },
                    "post": {
                        "operationId": "createTable",
                        "tags": ["Tables"],
                        "requestBody": { "required": true }
                    }
                },
                "/v1/tables/{id}": {
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "get": { "operationId": "getTableByID", "tags": ["Tables"] }
                }
            }
        });
        let idx = Index::from_spec(&spec);
        let tables = idx.get("tables").expect("tables group");
        assert_eq!(tables.len(), 3);
        let actions: Vec<&str> = tables.iter().map(|o| o.action.as_str()).collect();
        assert!(actions.contains(&"list"));
        assert!(actions.contains(&"create"));
        assert!(actions.contains(&"get-by-id"));

        let get_by_id = tables.iter().find(|o| o.action == "get-by-id").unwrap();
        assert_eq!(get_by_id.path_params.len(), 1);
        assert_eq!(get_by_id.path_params[0].name, "id");

        let list = tables.iter().find(|o| o.action == "list").unwrap();
        assert_eq!(list.query_params.len(), 1);
        assert_eq!(list.query_params[0].kind, ParamKind::Integer);

        let create = tables.iter().find(|o| o.action == "create").unwrap();
        assert!(create.has_body);
        assert!(create.body_required);
    }
}

#[cfg(test)]
mod real_spec_tests {
    use super::*;

    #[test]
    #[ignore]
    fn sandbox_spec_indexes() {
        let text = std::fs::read_to_string("tests/fixtures/sandbox-spec.json")
            .expect("fixture spec missing; run curl to populate");
        let spec: serde_json::Value = serde_json::from_str(&text).unwrap();
        let idx = Index::from_spec(&spec);
        assert!(idx.groups.len() >= 50, "got {} groups", idx.groups.len());
        let tables = idx.get("tables").expect("tables group");
        assert!(tables.len() >= 30, "tables had {} ops", tables.len());
        // Every action name must be unique within its group.
        for (group, ops) in &idx.groups {
            let mut names = std::collections::HashSet::new();
            for op in ops {
                assert!(
                    names.insert(op.action.clone()),
                    "duplicate action `{}` in group `{}`",
                    op.action,
                    group
                );
            }
        }
        // Print a couple actions for eyeballing.
        eprintln!(
            "tables actions: {:?}",
            tables
                .iter()
                .take(10)
                .map(|o| format!("{} {} {}", o.method, o.action, o.path))
                .collect::<Vec<_>>()
        );
    }
}
