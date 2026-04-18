//! `omd lineage <fqn>` — fetch upstream/downstream lineage for an entity.
//!
//! Renders as an ASCII tree, Mermaid, Graphviz DOT, or raw JSON.

use crate::client::OmdClient;
use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use crate::output::{self, OutputCtx};
use crate::util::entity;
use reqwest::Method;
use std::collections::{BTreeMap, BTreeSet};

#[derive(clap::Args, Debug)]
pub struct LineageArgs {
    /// Fully-qualified name of the root entity.
    pub fqn: String,

    /// Entity type hint (table, dashboard, ...). Resolved via search if omitted.
    #[arg(long)]
    pub r#type: Option<String>,

    /// Traverse upstream edges only.
    #[arg(long, conflicts_with = "down")]
    pub up: bool,

    /// Traverse downstream edges only.
    #[arg(long, conflicts_with = "up")]
    pub down: bool,

    /// Depth in each selected direction (default 2).
    #[arg(long, default_value_t = 2)]
    pub depth: u32,

    /// Output format.
    #[arg(long, value_parser = ["tree", "mermaid", "dot", "json"], default_value = "tree")]
    pub format: String,
}

#[derive(Clone, Debug)]
struct Node {
    id: String,
    name: String,
    entity_type: String,
}

pub async fn run(profile: &str, args: LineageArgs, ctx: &OutputCtx) -> CliResult<()> {
    let cfg = ResolvedConfig::load(profile)?;
    cfg.require_token()?;
    let client = OmdClient::new(&cfg)?;

    let entity_type = match args.r#type.clone() {
        Some(t) => t,
        None => entity::resolve_type(&client, &args.fqn).await?,
    };

    let (want_up, want_down) = match (args.up, args.down) {
        (true, false) => (true, false),
        (false, true) => (false, true),
        _ => (true, true),
    };

    let path = format!(
        "v1/lineage/{}/name/{}",
        urlencode(&entity_type),
        urlencode(&args.fqn)
    );
    let query = vec![
        (
            "upstreamDepth".to_string(),
            if want_up { args.depth } else { 0 }.to_string(),
        ),
        (
            "downstreamDepth".to_string(),
            if want_down { args.depth } else { 0 }.to_string(),
        ),
    ];

    let data = client.json(Method::GET, &path, &query, None).await?;

    if ctx.json || args.format == "json" {
        output::print_json(&data);
        return Ok(());
    }

    let graph = parse_graph(&data)?;
    match args.format.as_str() {
        "tree" => render_tree(&graph, want_up, want_down),
        "mermaid" => render_mermaid(&graph, want_up, want_down),
        "dot" => render_dot(&graph, want_up, want_down),
        other => {
            return Err(CliError::InvalidInput(format!(
                "unsupported format `{other}`"
            )))
        }
    }
    Ok(())
}

struct Graph {
    root: String,
    nodes: BTreeMap<String, Node>,
    upstream: BTreeMap<String, Vec<String>>,
    downstream: BTreeMap<String, Vec<String>>,
}

fn parse_graph(v: &serde_json::Value) -> CliResult<Graph> {
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();

    let root_ref = v.get("entity").ok_or_else(|| CliError::Api {
        status: 0,
        message: "lineage response missing `entity`".into(),
    })?;
    let root = node_from_ref(root_ref);
    let root_id = root.id.clone();
    nodes.insert(root_id.clone(), root);

    if let Some(arr) = v.get("nodes").and_then(|n| n.as_array()) {
        for r in arr {
            let n = node_from_ref(r);
            if !n.id.is_empty() {
                nodes.insert(n.id.clone(), n);
            }
        }
    }

    let mut upstream: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut downstream: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for e in v
        .get("downstreamEdges")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
    {
        if let Some((from, to)) = edge_pair(e) {
            downstream.entry(from).or_default().push(to);
        }
    }
    for e in v
        .get("upstreamEdges")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
    {
        if let Some((from, to)) = edge_pair(e) {
            upstream.entry(to).or_default().push(from);
        }
    }

    Ok(Graph {
        root: root_id,
        nodes,
        upstream,
        downstream,
    })
}

fn node_from_ref(v: &serde_json::Value) -> Node {
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let name = v
        .get("fullyQualifiedName")
        .or_else(|| v.get("name"))
        .or_else(|| v.get("displayName"))
        .and_then(|x| x.as_str())
        .unwrap_or("?")
        .to_string();
    let entity_type = v
        .get("type")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Node {
        id,
        name,
        entity_type,
    }
}

fn edge_pair(v: &serde_json::Value) -> Option<(String, String)> {
    let from = v.get("fromEntity")?.as_str()?.to_string();
    let to = v.get("toEntity")?.as_str()?.to_string();
    Some((from, to))
}

fn render_tree(g: &Graph, up: bool, down: bool) {
    let root_name = display_name(g, &g.root);
    println!("{root_name}");
    if up {
        let ups = g.upstream.get(&g.root).cloned().unwrap_or_default();
        print_branch(g, &ups, &mut BTreeSet::new(), "", Direction::Up);
    }
    if down {
        let downs = g.downstream.get(&g.root).cloned().unwrap_or_default();
        print_branch(g, &downs, &mut BTreeSet::new(), "", Direction::Down);
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
}

fn print_branch(
    g: &Graph,
    children: &[String],
    visited: &mut BTreeSet<String>,
    prefix: &str,
    dir: Direction,
) {
    let arrow = match dir {
        Direction::Up => "↑",
        Direction::Down => "↓",
    };
    for (i, child) in children.iter().enumerate() {
        let last = i + 1 == children.len();
        let branch = if last { "└── " } else { "├── " };
        let name = display_name(g, child);
        println!("{prefix}{branch}{arrow} {name}");
        if !visited.insert(child.clone()) {
            continue;
        }
        let next_prefix = format!("{prefix}{}", if last { "    " } else { "│   " });
        let next = match dir {
            Direction::Up => g.upstream.get(child).cloned().unwrap_or_default(),
            Direction::Down => g.downstream.get(child).cloned().unwrap_or_default(),
        };
        print_branch(g, &next, visited, &next_prefix, dir);
    }
}

fn render_mermaid(g: &Graph, up: bool, down: bool) {
    println!("graph LR");
    for (id, node) in &g.nodes {
        let label = format!("{} ({})", node.name, node.entity_type);
        println!("    {}[\"{}\"]", short(id), escape_mermaid(&label));
    }
    if up {
        for (to, froms) in &g.upstream {
            for from in froms {
                println!("    {} --> {}", short(from), short(to));
            }
        }
    }
    if down {
        for (from, tos) in &g.downstream {
            for to in tos {
                println!("    {} --> {}", short(from), short(to));
            }
        }
    }
}

fn render_dot(g: &Graph, up: bool, down: bool) {
    println!("digraph lineage {{");
    println!("    rankdir=LR;");
    println!("    node [shape=box, style=rounded];");
    for (id, node) in &g.nodes {
        let label = format!("{}\\n{}", node.name, node.entity_type);
        println!("    \"{}\" [label=\"{}\"];", short(id), escape_dot(&label));
    }
    if up {
        for (to, froms) in &g.upstream {
            for from in froms {
                println!("    \"{}\" -> \"{}\";", short(from), short(to));
            }
        }
    }
    if down {
        for (from, tos) in &g.downstream {
            for to in tos {
                println!("    \"{}\" -> \"{}\";", short(from), short(to));
            }
        }
    }
    println!("}}");
}

fn display_name(g: &Graph, id: &str) -> String {
    match g.nodes.get(id) {
        Some(n) if n.entity_type.is_empty() => n.name.clone(),
        Some(n) => format!("{} ({})", n.name, n.entity_type),
        None => format!("<{}>", short(id)),
    }
}

fn short(id: &str) -> String {
    let head: String = id.chars().take(8).collect();
    format!("n{}", head.replace('-', ""))
}

fn escape_mermaid(s: &str) -> String {
    s.replace('"', "&quot;")
}

fn escape_dot(s: &str) -> String {
    s.replace('"', "\\\"")
}

fn urlencode(s: &str) -> String {
    entity::urlencode_segment(s)
}
