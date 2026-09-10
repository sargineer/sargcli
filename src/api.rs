//! `/api` is the server describing itself. We cache it for a day, read the
//! validator's field rules from it, and notice when the version moves.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::Client;
use crate::config::{Config, Paths};
use crate::error::Result;

pub const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Serialize, Deserialize)]
struct Cached {
    fetched_at: u64,
    body: Value,
}

#[derive(Debug, Clone)]
pub struct ApiInfo {
    pub version: String,
    pub updated: String,
    pub endpoints: Vec<String>,
    pub note_fields: Value,
    pub raw: Value,
    /// True when this came from the network on this call.
    pub fresh: bool,
    pub cache_age_secs: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse(body: Value, fresh: bool, age: u64) -> ApiInfo {
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let endpoints = body
        .get("endpoints")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    ApiInfo {
        version: s("version"),
        updated: s("updated"),
        endpoints,
        note_fields: body.get("note_fields").cloned().unwrap_or(Value::Null),
        raw: body,
        fresh,
        cache_age_secs: age,
    }
}

/// Load `/api`, from cache when it is under a day old unless `force`.
/// Updates the last-seen version for `env` and prints a one-line drift
/// notice to stderr when the server has moved since the last look.
pub fn load(
    client: &Client,
    paths: &Paths,
    cfg: &mut Config,
    env: &str,
    force: bool,
) -> Result<ApiInfo> {
    let cache_file = paths.cache_dir.join("api.json");
    if !force {
        if let Ok(s) = fs::read_to_string(&cache_file) {
            if let Ok(c) = serde_json::from_str::<Cached>(&s) {
                let age = now().saturating_sub(c.fetched_at);
                if age < CACHE_TTL_SECS {
                    return Ok(parse(c.body, false, age));
                }
            }
        }
    }
    let body = client.get_json("/api")?;
    fs::create_dir_all(&paths.cache_dir)?;
    let cached = Cached {
        fetched_at: now(),
        body: body.clone(),
    };
    fs::write(&cache_file, serde_json::to_vec(&cached)?)?;
    let info = parse(body, true, 0);
    note_drift(&info, cfg, env, paths)?;
    Ok(info)
}

fn note_drift(info: &ApiInfo, cfg: &mut Config, env: &str, paths: &Paths) -> Result<()> {
    if info.version.is_empty() {
        return Ok(());
    }
    let (last, previous) = cfg.versions_mut(env);
    match last.clone().as_deref() {
        Some(seen) if seen == info.version => Ok(()),
        Some(seen) => {
            eprintln!(
                "sarg: sargineer moved {seen} → {} (updated {}). Run `sarg changelog` to see what changed.",
                info.version, info.updated
            );
            *previous = Some(seen.to_string());
            *last = Some(info.version.clone());
            cfg.save(paths)
        }
        None => {
            *last = Some(info.version.clone());
            cfg.save(paths)
        }
    }
}
