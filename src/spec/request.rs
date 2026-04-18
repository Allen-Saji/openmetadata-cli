//! Translate clap matches on a dynamic Operation into an HTTP request.

use crate::error::{CliError, CliResult};
use crate::spec::index::{Operation, Param, ParamKind};
use clap::ArgMatches;
use reqwest::Method;

pub struct BuiltRequest {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Option<serde_json::Value>,
}

pub fn build(op: &Operation, matches: &ArgMatches) -> CliResult<BuiltRequest> {
    let method = Method::from_bytes(op.method.as_bytes())
        .map_err(|_| CliError::InvalidInput(format!("invalid HTTP method `{}`", op.method)))?;

    let mut path = op.path.clone();
    for p in &op.path_params {
        let id = path_arg_id(&p.name);
        let val: &String = matches
            .get_one::<String>(&id)
            .ok_or_else(|| CliError::InvalidInput(format!("missing path argument `{}`", p.name)))?;
        let placeholder = format!("{{{}}}", p.name);
        path = path.replace(&placeholder, val);
    }

    let mut query = Vec::new();
    for q in &op.query_params {
        push_query(&mut query, q, matches)?;
    }

    let body = if op.has_body {
        match matches.get_one::<String>("body") {
            Some(raw) => Some(read_body(raw)?),
            None if op.body_required => {
                return Err(CliError::InvalidInput(format!(
                    "operation `{}` requires --body",
                    op.action
                )));
            }
            None => None,
        }
    } else {
        None
    };

    Ok(BuiltRequest {
        method,
        path,
        query,
        body,
    })
}

pub fn path_arg_id(name: &str) -> String {
    super::index::kebab(name)
}

pub fn query_arg_id(name: &str) -> String {
    format!("q-{}", super::index::kebab(name))
}

fn push_query(
    out: &mut Vec<(String, String)>,
    param: &Param,
    matches: &ArgMatches,
) -> CliResult<()> {
    let id = query_arg_id(&param.name);
    match param.kind {
        ParamKind::Array => {
            if let Some(values) = matches.get_many::<String>(&id) {
                for v in values {
                    out.push((param.name.clone(), v.clone()));
                }
            }
        }
        _ => {
            if let Some(v) = matches.get_one::<String>(&id) {
                out.push((param.name.clone(), v.clone()));
            }
        }
    }
    Ok(())
}

fn read_body(raw: &str) -> CliResult<serde_json::Value> {
    let text = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path)?
    } else if raw == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        raw.to_string()
    };
    Ok(serde_json::from_str(&text)?)
}
