use crate::config::ResolvedConfig;
use crate::error::{CliError, CliResult};
use reqwest::{header, Client, Method, RequestBuilder, Response};
use std::time::Duration;

const USER_AGENT: &str = concat!("omd/", env!("CARGO_PKG_VERSION"));

pub struct OmdClient {
    http: Client,
    host: String,
    token: Option<String>,
}

impl OmdClient {
    pub fn new(cfg: &ResolvedConfig) -> CliResult<Self> {
        let http = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .build()?;
        Ok(Self {
            http,
            host: cfg.host.clone(),
            token: cfg.token.clone(),
        })
    }

    fn url(&self, path: &str) -> String {
        let p = path.trim_start_matches('/');
        if p.starts_with("api/") {
            format!("{}/{}", self.host, p)
        } else {
            format!("{}/api/{}", self.host, p)
        }
    }

    fn authed(&self, req: RequestBuilder) -> RequestBuilder {
        if let Some(t) = &self.token {
            req.bearer_auth(t)
        } else {
            req
        }
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<serde_json::Value>,
    ) -> CliResult<Response> {
        let url = self.url(path);
        let mut req = self.http.request(method, &url).query(query);
        req = self.authed(req);
        if let Some(b) = body {
            req = req.header(header::CONTENT_TYPE, "application/json").json(&b);
        }
        let resp = req.send().await?;
        Ok(resp)
    }

    pub async fn json(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<serde_json::Value>,
    ) -> CliResult<serde_json::Value> {
        let resp = self.request(method, path, query, body).await?;
        read_json(resp).await
    }

    pub async fn get_json(&self, path: &str) -> CliResult<serde_json::Value> {
        self.json(Method::GET, path, &[], None).await
    }
}

pub async fn read_json(resp: Response) -> CliResult<serde_json::Value> {
    let status = resp.status();
    if status.is_success() {
        let v = resp.json::<serde_json::Value>().await?;
        return Ok(v);
    }
    let code = status.as_u16();
    let body = resp.text().await.unwrap_or_default();
    let message = match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => v
            .get("message")
            .and_then(|m| m.as_str())
            .map(String::from)
            .unwrap_or(body),
        Err(_) => body,
    };
    Err(CliError::Api {
        status: code,
        message,
    })
}
