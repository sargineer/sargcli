//! Thin HTTP client: bearer auth, the server's `hint` passed through, the
//! token never printed, and status codes mapped onto the exit-code table.

use std::time::Duration;

use serde_json::Value;

use crate::config::Resolved;
use crate::error::{Result, SargError};

pub struct Client {
    agent: ureq::Agent,
    base: String,
    token: Option<String>,
    debug: bool,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

#[allow(dead_code)] // the read-side verbs of phase 1 use the rest of this surface
impl Response {
    pub fn is_json(&self) -> bool {
        self.content_type.contains("json")
    }
    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.body).ok()
    }
}

#[allow(dead_code)] // the read-side verbs of phase 1 use the rest of this surface
impl Client {
    pub fn new(resolved: &Resolved, debug: bool) -> Client {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .user_agent(concat!("sarg/", env!("CARGO_PKG_VERSION")))
            .build();
        Client {
            agent,
            base: resolved.url.clone(),
            token: resolved.token.clone(),
            debug,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{}", self.base, path)
        }
    }

    fn prepare(&self, method: &str, path: &str, headers: &[(String, String)]) -> ureq::Request {
        let mut req = self.agent.request(method, &self.url(path));
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) {
            req = req.set("Accept", "application/json, text/markdown;q=0.9, */*;q=0.5");
        }
        for (k, v) in headers {
            req = req.set(k, v);
        }
        if self.debug {
            eprintln!(
                "sarg> {method} {} (auth: {})",
                self.url(path),
                if self.token.is_some() { "Bearer ****" } else { "none" }
            );
        }
        req
    }

    fn finish(&self, res: std::result::Result<ureq::Response, ureq::Error>) -> Result<Response> {
        let resp = match res {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => {
                return Err(SargError::Offline(format!(
                    "cannot reach {}: {}",
                    self.base,
                    t.message().unwrap_or("network error")
                )))
            }
        };
        let status = resp.status();
        let content_type = resp.content_type().to_string();
        let body = resp
            .into_string()
            .map_err(|e| SargError::other(anyhow::anyhow!("reading response: {e}")))?;
        if self.debug {
            eprintln!("sarg< {status} {content_type} {} bytes", body.len());
        }
        Ok(Response {
            status,
            content_type,
            body,
        })
    }

    /// Any method, optional body. Returns the response whatever the status;
    /// use [`Client::ok`] to turn non-2xx into an error.
    pub fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<Response> {
        let req = self.prepare(method, path, headers);
        let res = match body {
            Some(b) => req.send_string(b),
            None => req.call(),
        };
        self.finish(res)
    }

    pub fn get(&self, path: &str) -> Result<Response> {
        self.ok(self.send("GET", path, None, &[])?)
    }

    pub fn get_json(&self, path: &str) -> Result<Value> {
        let r = self.get(path)?;
        r.json().ok_or_else(|| {
            SargError::other(anyhow::anyhow!(
                "expected JSON from {path}, got {}",
                r.content_type
            ))
        })
    }

    pub fn post_json(&self, path: &str, body: &Value) -> Result<Response> {
        let headers = [("Content-Type".to_string(), "application/json".to_string())];
        self.ok(self.send("POST", path, Some(&body.to_string()), &headers)?)
    }

    pub fn post_empty(&self, path: &str) -> Result<Response> {
        self.ok(self.send("POST", path, Some(""), &[])?)
    }

    /// Binary download (model files). Errors carry the server's hint; a 401
    /// here means the token is missing, not that the file is gone.
    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let req = self.prepare("GET", path, &[]);
        match req.call() {
            Ok(resp) => {
                use std::io::Read;
                let mut buf = Vec::new();
                let mut reader = resp.into_reader();
                let mut limited = Read::by_ref(&mut reader).take(512 * 1024 * 1024);
                limited.read_to_end(&mut buf)?;
                Ok(buf)
            }
            Err(ureq::Error::Status(_, r)) => {
                let r = self.finish(Ok(r))?;
                Err(error_from(&r))
            }
            Err(ureq::Error::Transport(t)) => Err(SargError::Offline(format!(
                "cannot reach {}: {}",
                self.base,
                t.message().unwrap_or("network error")
            ))),
        }
    }

    /// Map a non-2xx response to the error table. 3xx is accepted: the
    /// server answers publish with a redirect meant for browsers.
    pub fn ok(&self, r: Response) -> Result<Response> {
        if r.status < 400 {
            return Ok(r);
        }
        Err(error_from(&r))
    }
}

/// Percent-encode a query value (spaces become `+`).
pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the error for a failed response, carrying the server's own hint.
pub fn error_from(r: &Response) -> SargError {
    let j = r.json();
    let pick = |keys: &[&str]| -> Option<String> {
        let j = j.as_ref()?;
        keys.iter()
            .find_map(|k| j.get(*k).and_then(|v| v.as_str()).map(str::to_string))
    };
    let mut message = pick(&["error", "detail", "message"]).unwrap_or_else(|| {
        let t = r.body.trim();
        if t.is_empty() {
            format!("HTTP {}", r.status)
        } else {
            t.chars().take(300).collect()
        }
    });
    let hint = pick(&["hint", "next"]);
    if let Some(j) = &j {
        // The validator reports field problems under a few different names.
        for k in ["fields", "errors", "problems", "missing"] {
            if let Some(v) = j.get(k) {
                message = format!("{message} — {k}: {v}");
            }
        }
    }
    match r.status {
        401 | 403 => SargError::Auth { message, hint },
        400 | 409 | 415 | 422 => SargError::Validation { message, hint },
        s => SargError::Server {
            status: s,
            message,
            hint,
        },
    }
}
