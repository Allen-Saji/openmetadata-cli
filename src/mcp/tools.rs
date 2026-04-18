//! Curated MCP tool set exposed by `omd mcp`.
//!
//! Tools are thin wrappers over the same `OmdClient` and util modules that
//! power the CLI subcommands. Each call is stateless: the tool rebuilds a
//! `ResolvedConfig` from env + `~/.omd/`, so MCP clients can keep a single
//! long-running server process while the underlying config evolves on disk.

use crate::client::{read_json, OmdClient};
use crate::config::ResolvedConfig;
use crate::error::CliError;
use crate::util::{csv as csv_util, entity};
use reqwest::Method;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::Deserialize;
use serde_json::{json, Value};

// ---------- server ----------

const DEFAULT_PROFILE: &str = "default";

#[derive(Clone)]
pub struct OmdMcp {
    #[allow(dead_code)] // field read by the tool_router macro via self.tool_router
    tool_router: ToolRouter<OmdMcp>,
    allow_raw: bool,
}

impl OmdMcp {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            allow_raw: std::env::var("OMD_MCP_ALLOW_RAW")
                .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
                .unwrap_or(false),
        }
    }

    fn client(&self) -> Result<OmdClient, McpError> {
        let profile = std::env::var("OMD_PROFILE").unwrap_or_else(|_| DEFAULT_PROFILE.to_string());
        let cfg = ResolvedConfig::load(&profile).map_err(to_mcp)?;
        cfg.require_token().map_err(to_mcp)?;
        OmdClient::new(&cfg).map_err(to_mcp)
    }
}

impl Default for OmdMcp {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- tool input schemas ----------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchInput {
    /// Full-text search query (Lucene syntax supported).
    pub query: String,
    /// Entity index: table, dashboard, pipeline, topic, user, ... Defaults to all.
    #[serde(default)]
    pub index: Option<String>,
    /// Max results (default 10).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FqnInput {
    /// Fully-qualified name of the entity.
    pub fqn: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DescribeInput {
    pub fqn: String,
    #[serde(default)]
    pub entity_type: Option<String>,
    /// Comma-separated fields to include (default: owners,tags,followers).
    #[serde(default)]
    pub fields: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LineageInput {
    pub fqn: String,
    #[serde(default)]
    pub entity_type: Option<String>,
    /// Upstream traversal depth (default 2).
    #[serde(default)]
    pub upstream_depth: Option<u32>,
    /// Downstream traversal depth (default 2).
    #[serde(default)]
    pub downstream_depth: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DirectionalLineageInput {
    pub fqn: String,
    #[serde(default)]
    pub entity_type: Option<String>,
    /// Depth in the selected direction (default 2).
    #[serde(default)]
    pub depth: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateDescriptionInput {
    pub fqn: String,
    pub description: String,
    #[serde(default)]
    pub entity_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TagInput {
    pub fqn: String,
    pub tag_fqn: String,
    #[serde(default)]
    pub entity_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GlossaryInput {
    pub fqn: String,
    pub term_fqn: String,
    #[serde(default)]
    pub entity_type: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QualityListInput {
    /// Scope to a table FQN (optional).
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TestResultsInput {
    pub test_case_fqn: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CsvExportInput {
    /// Entity type: table, database, databaseSchema, glossary, ...
    pub entity_type: String,
    pub fqn: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CsvImportInput {
    pub entity_type: String,
    pub fqn: String,
    /// CSV payload as a string.
    pub csv: String,
    /// Apply changes (default false = dry-run).
    #[serde(default)]
    pub apply: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RawRequestInput {
    /// HTTP method (GET, POST, PUT, PATCH, DELETE).
    pub method: String,
    /// Path relative to the API root, e.g. `v1/tables` or `/api/v1/tables`.
    pub path: String,
    /// Query params as a list of `key=value` strings.
    #[serde(default)]
    pub query: Vec<String>,
    /// Optional JSON body.
    #[serde(default)]
    pub body: Option<Value>,
}

// ---------- tools ----------

#[tool_router]
impl OmdMcp {
    #[tool(description = "Search the catalog using Lucene-style queries. Returns entity hits.")]
    async fn search(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let idx = index_prefix(input.index.as_deref().unwrap_or("all"));
        let q = vec![
            ("q".into(), input.query),
            ("index".into(), format!("{idx}_search_index")),
            ("from".into(), "0".into()),
            ("size".into(), input.limit.unwrap_or(10).to_string()),
        ];
        let v = client
            .json(Method::GET, "v1/search/query", &q, None)
            .await
            .map_err(to_mcp)?;
        json_tool_result(&v)
    }

    #[tool(description = "Resolve an FQN to its entity type and id via the search index.")]
    async fn resolve_fqn(
        &self,
        Parameters(input): Parameters<FqnInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let t = entity::resolve_type(&client, &input.fqn)
            .await
            .map_err(to_mcp)?;
        let (_ty, v) = entity::fetch_by_fqn(&client, &input.fqn, Some(&t), None)
            .await
            .map_err(to_mcp)?;
        let id = entity::entity_id(&v).unwrap_or_default();
        json_tool_result(&json!({ "entity_type": t, "id": id, "fqn": input.fqn }))
    }

    #[tool(description = "Fetch a full entity payload by FQN. Use `fields` for tags/columns/etc.")]
    async fn describe_entity(
        &self,
        Parameters(input): Parameters<DescribeInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let fields = input.fields.as_deref().unwrap_or("owners,tags,followers");
        let (_ty, v) = entity::fetch_by_fqn(
            &client,
            &input.fqn,
            input.entity_type.as_deref(),
            Some(fields),
        )
        .await
        .map_err(to_mcp)?;
        json_tool_result(&v)
    }

    #[tool(description = "Get lineage graph (nodes + upstream/downstream edges) for an entity.")]
    async fn get_lineage(
        &self,
        Parameters(input): Parameters<LineageInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let ty = resolve_or_hint(&client, &input.fqn, input.entity_type.as_deref()).await?;
        let path = format!(
            "v1/lineage/{}/name/{}",
            entity::urlencode_segment(&ty),
            entity::urlencode_segment(&input.fqn)
        );
        let query = vec![
            (
                "upstreamDepth".into(),
                input.upstream_depth.unwrap_or(2).to_string(),
            ),
            (
                "downstreamDepth".into(),
                input.downstream_depth.unwrap_or(2).to_string(),
            ),
        ];
        let v = client
            .json(Method::GET, &path, &query, None)
            .await
            .map_err(to_mcp)?;
        json_tool_result(&v)
    }

    #[tool(description = "List upstream ancestors of an entity as a flat array of FQNs + types.")]
    async fn list_upstream(
        &self,
        Parameters(input): Parameters<DirectionalLineageInput>,
    ) -> Result<CallToolResult, McpError> {
        let flat = flat_lineage(self, input, true).await?;
        json_tool_result(&flat)
    }

    #[tool(
        description = "List downstream descendants of an entity as a flat array of FQNs + types."
    )]
    async fn list_downstream(
        &self,
        Parameters(input): Parameters<DirectionalLineageInput>,
    ) -> Result<CallToolResult, McpError> {
        let flat = flat_lineage(self, input, false).await?;
        json_tool_result(&flat)
    }

    #[tool(description = "Replace the description field on an entity via JSON patch.")]
    async fn update_description(
        &self,
        Parameters(input): Parameters<UpdateDescriptionInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let (ty, current) = entity::fetch_by_fqn(
            &client,
            &input.fqn,
            input.entity_type.as_deref(),
            Some("tags"),
        )
        .await
        .map_err(to_mcp)?;
        let id = entity::entity_id(&current)
            .ok_or_else(|| to_mcp(CliError::NotFound(input.fqn.clone())))?;
        let op = if current.get("description").is_some() {
            "replace"
        } else {
            "add"
        };
        let patch = json!([
            { "op": op, "path": "/description", "value": input.description }
        ]);
        let path = format!("{}/{}", entity::endpoint_for_type(&ty), id);
        let v = client.json_patch(&path, &patch).await.map_err(to_mcp)?;
        json_tool_result(&v)
    }

    #[tool(description = "Add a classification tag (by FQN) to an entity.")]
    async fn add_tag(
        &self,
        Parameters(input): Parameters<TagInput>,
    ) -> Result<CallToolResult, McpError> {
        mutate_tags(
            self,
            &input.fqn,
            input.entity_type.as_deref(),
            Some(&input.tag_fqn),
            None,
        )
        .await
    }

    #[tool(description = "Remove a classification tag (by FQN) from an entity.")]
    async fn remove_tag(
        &self,
        Parameters(input): Parameters<TagInput>,
    ) -> Result<CallToolResult, McpError> {
        mutate_tags(
            self,
            &input.fqn,
            input.entity_type.as_deref(),
            None,
            Some(&input.tag_fqn),
        )
        .await
    }

    #[tool(description = "Attach a glossary term (by FQN) to an entity.")]
    async fn assign_glossary_term(
        &self,
        Parameters(input): Parameters<GlossaryInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let (ty, current) = entity::fetch_by_fqn(
            &client,
            &input.fqn,
            input.entity_type.as_deref(),
            Some("tags"),
        )
        .await
        .map_err(to_mcp)?;
        let id = entity::entity_id(&current)
            .ok_or_else(|| to_mcp(CliError::NotFound(input.fqn.clone())))?;

        let mut tags: Vec<Value> = current
            .get("tags")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        if !tags.iter().any(|t| {
            t.get("tagFQN")
                .and_then(|v| v.as_str())
                .map(|s| s == input.term_fqn)
                .unwrap_or(false)
        }) {
            tags.push(json!({
                "tagFQN": input.term_fqn,
                "source": "Glossary",
                "labelType": "Manual",
                "state": "Confirmed",
            }));
        }
        let op = if current.get("tags").is_some() {
            "replace"
        } else {
            "add"
        };
        let patch = json!([{ "op": op, "path": "/tags", "value": tags }]);
        let path = format!("{}/{}", entity::endpoint_for_type(&ty), id);
        let v = client.json_patch(&path, &patch).await.map_err(to_mcp)?;
        json_tool_result(&v)
    }

    #[tool(description = "List data quality test cases, optionally scoped to a table FQN.")]
    async fn list_quality_tests(
        &self,
        Parameters(input): Parameters<QualityListInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let mut q: Vec<(String, String)> = vec![
            ("limit".into(), input.limit.unwrap_or(25).to_string()),
            ("fields".into(), "testSuite,entityLink".into()),
        ];
        if let Some(table) = input.table {
            q.push(("entityLink".into(), format!("<#E::table::{table}>")));
        }
        let v = client
            .json(Method::GET, "v1/dataQuality/testCases", &q, None)
            .await
            .map_err(to_mcp)?;
        json_tool_result(&v)
    }

    #[tool(description = "Fetch recent test case results by test case FQN.")]
    async fn get_test_results(
        &self,
        Parameters(input): Parameters<TestResultsInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let path = format!(
            "v1/dataQuality/testCases/testCaseResults/{}",
            entity::urlencode_segment(&input.test_case_fqn)
        );
        let q = vec![("limit".into(), input.limit.unwrap_or(10).to_string())];
        let v = client
            .json(Method::GET, &path, &q, None)
            .await
            .map_err(to_mcp)?;
        json_tool_result(&v)
    }

    #[tool(
        description = "Export entity metadata as CSV. Supported: table, database, databaseSchema, glossary, glossaryTerm, team, user, databaseService, securityService, testCase."
    )]
    async fn export_csv(
        &self,
        Parameters(input): Parameters<CsvExportInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let collection = csv_util::collection_for(&input.entity_type).map_err(to_mcp)?;
        let path = format!(
            "{collection}/name/{}/export",
            entity::urlencode_segment(&input.fqn)
        );
        let v = client
            .json(Method::GET, &path, &[], None)
            .await
            .map_err(to_mcp)?;
        let csv_text = v.as_str().unwrap_or_default().to_string();
        Ok(CallToolResult::success(vec![Content::text(csv_text)]))
    }

    #[tool(
        description = "Import entity metadata from a CSV string. Dry-run unless `apply` is true."
    )]
    async fn import_csv(
        &self,
        Parameters(input): Parameters<CsvImportInput>,
    ) -> Result<CallToolResult, McpError> {
        let client = self.client()?;
        let collection = csv_util::collection_for(&input.entity_type).map_err(to_mcp)?;
        let path = format!(
            "{collection}/name/{}/import",
            entity::urlencode_segment(&input.fqn)
        );
        let q = vec![("dryRun".into(), (!input.apply.unwrap_or(false)).to_string())];
        let resp = client
            .request_text(Method::PUT, &path, &q, input.csv)
            .await
            .map_err(to_mcp)?;
        let v = read_json(resp).await.map_err(to_mcp)?;
        json_tool_result(&v)
    }

    #[tool(
        description = "Escape hatch: raw API call. Only enabled when OMD_MCP_ALLOW_RAW=1 on the server."
    )]
    async fn raw_request(
        &self,
        Parameters(input): Parameters<RawRequestInput>,
    ) -> Result<CallToolResult, McpError> {
        if !self.allow_raw {
            return Err(McpError::invalid_request(
                "raw_request disabled — start omd mcp with OMD_MCP_ALLOW_RAW=1 to enable"
                    .to_string(),
                None,
            ));
        }
        let client = self.client()?;
        let method = Method::from_bytes(input.method.to_uppercase().as_bytes()).map_err(|_| {
            to_mcp(CliError::InvalidInput(format!(
                "invalid method `{}`",
                input.method
            )))
        })?;
        let mut q: Vec<(String, String)> = Vec::new();
        for item in &input.query {
            let (k, v) = item.split_once('=').ok_or_else(|| {
                to_mcp(CliError::InvalidInput(format!(
                    "query `{item}` not key=value"
                )))
            })?;
            q.push((k.to_string(), v.to_string()));
        }
        let v = client
            .json(method, &input.path, &q, input.body)
            .await
            .map_err(to_mcp)?;
        json_tool_result(&v)
    }
}

#[tool_handler]
impl ServerHandler for OmdMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "OpenMetadata CLI exposed as MCP. Configure auth via ~/.omd/config.toml + credentials or OMD_HOST/OMD_TOKEN env vars before calling tools."
                .into(),
        );
        info
    }
}

// ---------- helpers ----------

fn to_mcp(e: CliError) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn json_tool_result(v: &Value) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn index_prefix(idx: &str) -> &'static str {
    match idx {
        "all" | "" => "all",
        "table" | "tables" => "table",
        "dashboard" | "dashboards" => "dashboard",
        "pipeline" | "pipelines" => "pipeline",
        "topic" | "topics" => "topic",
        "mlmodel" | "mlmodels" => "mlmodel",
        "container" | "containers" => "container",
        "glossary" | "glossaries" | "glossaryTerm" => "glossary",
        "tag" | "tags" => "tag",
        "user" | "users" => "user",
        "team" | "teams" => "team",
        _ => "all",
    }
}

async fn resolve_or_hint(
    client: &OmdClient,
    fqn: &str,
    hint: Option<&str>,
) -> Result<String, McpError> {
    match hint {
        Some(t) => Ok(t.to_string()),
        None => entity::resolve_type(client, fqn).await.map_err(to_mcp),
    }
}

async fn mutate_tags(
    server: &OmdMcp,
    fqn: &str,
    hint: Option<&str>,
    add: Option<&str>,
    remove: Option<&str>,
) -> Result<CallToolResult, McpError> {
    let client = server.client()?;
    let (ty, current) = entity::fetch_by_fqn(&client, fqn, hint, Some("tags"))
        .await
        .map_err(to_mcp)?;
    let id = entity::entity_id(&current).ok_or_else(|| to_mcp(CliError::NotFound(fqn.into())))?;

    let mut tags: Vec<Value> = current
        .get("tags")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(r) = remove {
        tags.retain(|t| t.get("tagFQN").and_then(|v| v.as_str()) != Some(r));
    }
    if let Some(a) = add {
        if !tags
            .iter()
            .any(|t| t.get("tagFQN").and_then(|v| v.as_str()) == Some(a))
        {
            tags.push(json!({
                "tagFQN": a,
                "source": "Classification",
                "labelType": "Manual",
                "state": "Confirmed",
            }));
        }
    }
    let op = if current.get("tags").is_some() {
        "replace"
    } else {
        "add"
    };
    let patch = json!([{ "op": op, "path": "/tags", "value": tags }]);
    let path = format!("{}/{}", entity::endpoint_for_type(&ty), id);
    let v = client.json_patch(&path, &patch).await.map_err(to_mcp)?;
    json_tool_result(&v)
}

async fn flat_lineage(
    server: &OmdMcp,
    input: DirectionalLineageInput,
    upstream: bool,
) -> Result<Value, McpError> {
    let client = server.client()?;
    let ty = resolve_or_hint(&client, &input.fqn, input.entity_type.as_deref()).await?;
    let path = format!(
        "v1/lineage/{}/name/{}",
        entity::urlencode_segment(&ty),
        entity::urlencode_segment(&input.fqn)
    );
    let depth = input.depth.unwrap_or(2).to_string();
    let query = vec![
        (
            "upstreamDepth".into(),
            if upstream { depth.clone() } else { "0".into() },
        ),
        (
            "downstreamDepth".into(),
            if upstream { "0".into() } else { depth },
        ),
    ];
    let v = client
        .json(Method::GET, &path, &query, None)
        .await
        .map_err(to_mcp)?;
    let nodes = v
        .get("nodes")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();
    let list: Vec<Value> = nodes
        .iter()
        .map(|n| {
            json!({
                "fqn": n.get("fullyQualifiedName").and_then(|v| v.as_str()),
                "type": n.get("type").and_then(|v| v.as_str()),
                "id": n.get("id").and_then(|v| v.as_str()),
            })
        })
        .collect();
    Ok(json!({ "direction": if upstream { "upstream" } else { "downstream" }, "nodes": list }))
}
