//! `sarg init`: record this machine's host tags and, if given on the
//! command line, the token and URL, then confirm with the server. The
//! probe is the one start.md prescribes — file reads only, no `nvidia-smi`,
//! no `lspci`. The token is never read from any agent's own settings.

use std::fs;

use serde_json::json;

use super::Ctx;
use crate::cli::InitArgs;
use crate::config::{self, TokenSource};
use crate::error::Result;
use crate::output;

pub fn run(ctx: &mut Ctx, args: &InitArgs) -> Result<()> {
    let mut changed: Vec<&str> = Vec::new();

    if let Some(u) = &ctx.cli.url {
        ctx.cfg.url = Some(u.trim_end_matches('/').to_string());
        changed.push("url");
    }
    if ctx.resolved.token_source == TokenSource::Flag {
        ctx.cfg.token = ctx.resolved.token.clone();
        changed.push("token");
    }
    if !args.no_probe {
        let tags = probe_host_tags();
        if !tags.is_empty() && tags != ctx.cfg.host_tags {
            ctx.cfg.host_tags = tags;
            changed.push("host_tags");
        }
    }

    ctx.cfg.save(&ctx.paths)?;
    fs::create_dir_all(&ctx.paths.pending_dir)?;
    fs::create_dir_all(&ctx.paths.cache_dir)?;

    // Re-resolve so the check below uses what we just saved.
    ctx.resolved = config::resolve(ctx.cli, &ctx.cfg)?;
    ctx.client = crate::client::Client::new(&ctx.resolved, ctx.cli.debug);

    let mut account = json!(null);
    let mut check = "skipped".to_string();
    if !args.offline {
        if ctx.resolved.token.is_some() {
            match ctx.client.get_json("/me") {
                Ok(me) => {
                    check = format!(
                        "signed in as {}",
                        me.get("handle").and_then(|h| h.as_str()).unwrap_or("?")
                    );
                    account = me;
                }
                Err(e) => check = format!("token rejected: {e}"),
            }
        } else {
            check = "no token yet".into();
        }
    }

    let agent = crate::agent::detect();
    if ctx.json() {
        output::json(&json!({
            "config": ctx.paths.config_file,
            "changed": changed,
            "host_tags": ctx.cfg.host_tags,
            "server": ctx.resolved.url,
            "agent": agent.name(),
            "check": check,
            "account": account,
        }));
    } else {
        output::kv("config", &ctx.paths.config_file.display().to_string());
        let changed_s = if changed.is_empty() {
            "nothing".to_string()
        } else {
            changed.join(", ")
        };
        output::kv("changed", &changed_s);
        output::kv("host_tags", &ctx.cfg.host_tags.join(", "));
        output::kv("server", &ctx.resolved.url);
        output::kv("agent", agent.name());
        output::kv("check", &check);
        if ctx.resolved.token.is_none() {
            println!();
            println!("next: sarg config set token <token>  ·  no account yet → sarg doc start");
        }
    }
    Ok(())
}

/// os, arch, distro-major, and the first NVIDIA GPU if the driver exposes one.
pub fn probe_host_tags() -> Vec<String> {
    let mut tags: Vec<String> = vec![
        std::env::consts::OS.to_string(),
        std::env::consts::ARCH.to_string(),
    ];
    if let Ok(rel) = fs::read_to_string("/etc/os-release") {
        let mut id = String::new();
        let mut ver = String::new();
        for line in rel.lines() {
            if let Some(v) = line.strip_prefix("ID=") {
                id = v.trim_matches('"').to_string();
            } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
                ver = v.trim_matches('"').split('.').next().unwrap_or("").to_string();
            }
        }
        if !id.is_empty() {
            tags.push(if ver.is_empty() { id } else { format!("{id}-{ver}") });
        }
    }
    if let Ok(dirs) = fs::read_dir("/proc/driver/nvidia/gpus") {
        let mut infos: Vec<_> = dirs.flatten().map(|d| d.path().join("information")).collect();
        infos.sort();
        if let Some(info) = infos.first() {
            tags.push("nvidia".into());
            if let Ok(s) = fs::read_to_string(info) {
                if let Some(m) = s
                    .lines()
                    .find_map(|l| l.strip_prefix("Model:").map(|m| m.trim().to_string()))
                {
                    tags.push(m);
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    tags.retain(|t| !t.is_empty() && seen.insert(t.clone()));
    tags
}
