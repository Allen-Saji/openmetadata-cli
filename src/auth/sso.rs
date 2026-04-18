//! OIDC authorization-code-with-PKCE login against the OpenMetadata server's
//! configured identity provider.
//!
//! The flow is:
//!   1. Read `{host}/api/v1/system/config/auth` to learn provider + authority.
//!   2. Discover `{authority}/.well-known/openid-configuration`.
//!   3. Generate a PKCE verifier + S256 challenge.
//!   4. Bind 127.0.0.1:0, open the browser at the authorize endpoint.
//!   5. Accept one GET on /callback, validate `state`, pull `code`.
//!   6. POST the code to the token endpoint with the verifier.
//!   7. Hand back the id_token (or access_token as a fallback).

use crate::error::{CliError, CliResult};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

pub const DEFAULT_SCOPES: &str = "openid email profile";
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);

/// Auth configuration discovered from the OpenMetadata server.
#[derive(Debug, Clone)]
pub struct OmAuthConfig {
    pub provider: String,
    pub authority: String,
    pub client_id: String,
}

impl OmAuthConfig {
    pub fn from_value(v: &serde_json::Value) -> CliResult<Self> {
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    CliError::Config(format!(
                        "auth config missing `{k}` — server may not have SSO enabled"
                    ))
                })
        };
        Ok(Self {
            provider: s("provider")?,
            authority: s("authority")?,
            client_id: s("clientId")?,
        })
    }
}

/// OIDC endpoints discovered from `.well-known/openid-configuration`.
#[derive(Debug, Clone)]
pub struct OidcEndpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
}

impl OidcEndpoints {
    pub fn from_value(v: &serde_json::Value) -> CliResult<Self> {
        let s = |k: &str| {
            v.get(k)
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| CliError::Config(format!("openid-configuration missing `{k}`")))
        };
        Ok(Self {
            authorization_endpoint: s("authorization_endpoint")?,
            token_endpoint: s("token_endpoint")?,
        })
    }
}

/// PKCE pair: the random verifier and its S256 challenge.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// Generate a fresh verifier (43-char URL-safe b64 of 32 random bytes) and
    /// the matching S256 challenge.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(digest);
        Self {
            verifier,
            challenge,
        }
    }
}

pub fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Build the provider authorize URL.
pub fn build_authorize_url(
    endpoints: &OidcEndpoints,
    client_id: &str,
    redirect_uri: &str,
    scopes: &str,
    state: &str,
    pkce: &Pkce,
) -> CliResult<String> {
    let mut url = url::Url::parse(&endpoints.authorization_endpoint)
        .map_err(|e| CliError::Config(format!("bad authorize URL: {e}")))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", scopes)
        .append_pair("state", state)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url.to_string())
}

/// Bind a local listener on a random port and return (listener, redirect_uri).
pub fn bind_loopback() -> CliResult<(TcpListener, String)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| CliError::Other(anyhow::anyhow!("bind loopback: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| CliError::Other(anyhow::anyhow!("local_addr: {e}")))?
        .port();
    let uri = format!("http://127.0.0.1:{port}/callback");
    Ok((listener, uri))
}

/// Wait for a single GET on `/callback?code=...&state=...` and return both.
pub fn accept_callback(listener: TcpListener, timeout: Duration) -> CliResult<CallbackParams> {
    listener
        .set_nonblocking(false)
        .map_err(|e| CliError::Other(anyhow::anyhow!("set blocking: {e}")))?;
    let addr: SocketAddr = listener
        .local_addr()
        .map_err(|e| CliError::Other(anyhow::anyhow!("local_addr: {e}")))?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(CliError::Other(anyhow::anyhow!(
                "timed out waiting for browser callback on {addr}"
            )));
        }
        listener
            .set_nonblocking(true)
            .map_err(|e| CliError::Other(anyhow::anyhow!("set_nonblocking: {e}")))?;
        match listener.accept() {
            Ok((stream, _)) => return handle_callback(stream),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(CliError::Other(anyhow::anyhow!("accept: {e}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

fn handle_callback(mut stream: TcpStream) -> CliResult<CallbackParams> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| CliError::Other(anyhow::anyhow!("set_read_timeout: {e}")))?;
    // Read just enough to capture the request line; browsers send the full
    // request quickly, so a modest buffer is plenty.
    let mut buf = [0u8; 8192];
    let n = stream
        .read(&mut buf)
        .map_err(|e| CliError::Other(anyhow::anyhow!("read callback: {e}")))?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    let params = parse_request_line(first_line);

    let body = render_response(&params);
    let _ = stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    );
    let _ = stream.flush();
    Ok(params)
}

fn render_response(params: &CallbackParams) -> String {
    if let Some(err) = &params.error {
        let desc = params.error_description.as_deref().unwrap_or("");
        format!(
            "<!doctype html><html><body style=\"font-family:system-ui;padding:2rem\">\
             <h2>Login failed</h2><p><strong>{err}</strong></p><p>{desc}</p>\
             <p>You can close this tab and try again in the terminal.</p></body></html>"
        )
    } else {
        "<!doctype html><html><body style=\"font-family:system-ui;padding:2rem\">\
         <h2>Login complete</h2><p>You can close this tab and return to the terminal.</p>\
         </body></html>"
            .to_string()
    }
}

pub fn parse_request_line(line: &str) -> CallbackParams {
    // Expected: "GET /callback?code=...&state=... HTTP/1.1"
    let mut parts = line.split_whitespace();
    let _method = parts.next();
    let path = parts.next().unwrap_or("");
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut out = CallbackParams {
        code: None,
        state: None,
        error: None,
        error_description: None,
    };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let decoded = percent_decode(v);
        match k {
            "code" => out.code = Some(decoded),
            "state" => out.state = Some(decoded),
            "error" => out.error = Some(decoded),
            "error_description" => out.error_description = Some(decoded),
            _ => {}
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        if b == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(((hi << 4) | lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_and_challenge_roundtrip() {
        let p = Pkce::generate();
        assert!(p.verifier.len() >= 43, "verifier too short: {}", p.verifier);
        assert!(p.verifier.len() <= 128);
        // Recompute the challenge to confirm S256 was used.
        let digest = Sha256::digest(p.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(digest);
        assert_eq!(expected, p.challenge);
    }

    #[test]
    fn two_pkces_differ() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_ne!(a.verifier, b.verifier);
    }

    #[test]
    fn authorize_url_includes_params() {
        let endpoints = OidcEndpoints {
            authorization_endpoint: "https://idp.example.com/authorize".into(),
            token_endpoint: "https://idp.example.com/token".into(),
        };
        let pkce = Pkce {
            verifier: "v1".into(),
            challenge: "chal".into(),
        };
        let url = build_authorize_url(
            &endpoints,
            "client-abc",
            "http://127.0.0.1:5555/callback",
            "openid email",
            "state-xyz",
            &pkce,
        )
        .unwrap();
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=client-abc"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=state-xyz"));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("scope=openid"));
    }

    #[test]
    fn parses_request_line() {
        let p = parse_request_line("GET /callback?code=abc&state=xyz HTTP/1.1");
        assert_eq!(p.code.as_deref(), Some("abc"));
        assert_eq!(p.state.as_deref(), Some("xyz"));
    }

    #[test]
    fn parses_url_encoded_state() {
        let p = parse_request_line("GET /callback?code=c%2Bd&state=s%2Fx HTTP/1.1");
        assert_eq!(p.code.as_deref(), Some("c+d"));
        assert_eq!(p.state.as_deref(), Some("s/x"));
    }

    #[test]
    fn surfaces_provider_errors() {
        let p = parse_request_line(
            "GET /callback?error=access_denied&error_description=User%20cancelled HTTP/1.1",
        );
        assert_eq!(p.error.as_deref(), Some("access_denied"));
        assert_eq!(p.error_description.as_deref(), Some("User cancelled"));
    }

    #[test]
    fn auth_config_parses_minimal_fields() {
        let v = serde_json::json!({
            "provider": "google",
            "authority": "https://accounts.google.com",
            "clientId": "abc.apps.googleusercontent.com"
        });
        let c = OmAuthConfig::from_value(&v).unwrap();
        assert_eq!(c.provider, "google");
        assert_eq!(c.client_id, "abc.apps.googleusercontent.com");
    }

    #[test]
    fn auth_config_errors_on_missing_fields() {
        let v = serde_json::json!({ "provider": "google" });
        assert!(OmAuthConfig::from_value(&v).is_err());
    }

    #[test]
    fn loopback_roundtrip_extracts_code_and_state() {
        use std::net::TcpStream;
        let (listener, redirect_uri) = bind_loopback().unwrap();
        let addr = listener.local_addr().unwrap();
        // Drive the callback from a separate thread: send a GET, read the
        // response, and exit so the server can finish.
        let sim = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut s = TcpStream::connect(addr).unwrap();
            s.write_all(
                b"GET /callback?code=my-code&state=my-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .unwrap();
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        });

        let params = accept_callback(listener, Duration::from_secs(3)).unwrap();
        let response = sim.join().unwrap();

        assert_eq!(params.code.as_deref(), Some("my-code"));
        assert_eq!(params.state.as_deref(), Some("my-state"));
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("Login complete"));
        assert!(redirect_uri.starts_with("http://127.0.0.1:"));
    }
}
