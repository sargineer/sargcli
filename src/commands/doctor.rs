//! `sarg doctor`: is the install healthy, and are stale pointers lying to
//! the next agent? The stale-pointer class (localhost:8093, mywarehouse,
//! "warehouse") caused real misfires; this finds them once.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::Ctx;
use crate::api;
use crate::error::Result;
use crate::render::{self, good, warn};

// Unambiguous pointers to the torn-down local server or the old tooling.
// The bare word "warehouse" is intentionally not here — the memory that
// says *not* to use it would trip it.
const STALE: [&str; 4] = ["localhost:8093", ":8093", "mywarehouse", "SARGINEER_URL"];

fn scan_dir_for_stale(root: &Path, hits: &mut Vec<String>) {
    let mut stack = vec![root.to_path_buf()];
    let mut budget = 4000;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if budget == 0 {
                return;
            }
            budget -= 1;
            let p = e.path();
            if p.is_dir() {
                let name = e.file_name();
                let n = name.to_string_lossy();
                if n == "node_modules" || n == ".git" || n == "target" {
                    continue;
                }
                stack.push(p);
            } else if p.extension().map(|x| x == "md" || x == "json" || x == "toml").unwrap_or(false) {
                if let Ok(text) = fs::read_to_string(&p) {
                    let low = text.to_lowercase();
                    for s in STALE {
                        if low.contains(&s.to_lowercase()) {
                            hits.push(format!("{}: {s}", p.display()));
                        }
                    }
                }
            }
        }
    }
}

pub fn run(ctx: &mut Ctx) -> Result<()> {
    let mut ok: Vec<String> = Vec::new();
    let mut probs: Vec<String> = Vec::new();

    // Which server
    if ctx.resolved.is_prod() {
        ok.push(format!("env prod · {}", ctx.resolved.url));
    } else {
        probs.push(format!(
            "env {} · {} — not sargineer.com; sarg env prod switches back",
            ctx.resolved.env, ctx.resolved.url
        ));
    }

    // Token + config
    match &ctx.resolved.token {
        Some(_) => ok.push(format!("token present ({})", ctx.resolved.token_source)),
        None => probs.push("no token — sarg config set token <token>".into()),
    }
    if ctx.paths.config_file.exists() {
        ok.push(format!("config at {}", ctx.paths.config_file.display()));
    } else {
        probs.push("no config file — run sarg init".into());
    }
    if ctx.cfg.host_tags.is_empty() {
        probs.push("no host_tags — sarg init to fill them".into());
    } else {
        ok.push(format!("host_tags: {}", ctx.cfg.host_tags.join(", ")));
    }
    if ctx.cfg.deny_terms.is_empty() {
        probs.push("deny_terms empty — set names that must never be sent: sarg config set deny_terms \"a,b\"".into());
    } else {
        ok.push(format!("deny_terms: {} set", ctx.cfg.deny_terms.len()));
    }

    // Agent
    let agent = crate::agent::detect();
    ok.push(format!("agent: {}", agent.name()));

    // Account + server drift
    let mut server_line = String::new();
    if ctx.resolved.token.is_some() {
        match ctx.client.get_json("/me") {
            Ok(me) => ok.push(format!(
                "signed in as {}",
                me.get("handle").and_then(|h| h.as_str()).unwrap_or("?")
            )),
            Err(e) => probs.push(format!("token rejected: {e}")),
        }
    }
    let paths = ctx.paths.clone();
    let env = ctx.resolved.env.clone();
    if let Ok(info) = api::load(&ctx.client, &paths, &mut ctx.cfg, &env, false) {
        server_line = format!("server {} (updated {})", info.version, info.updated);
        ok.push(server_line.clone());
    }

    // Stale pointers in Claude's memory + instructions
    let mut stale: Vec<String> = Vec::new();
    for rel in [".claude/CLAUDE.md", ".claude/projects", "CLAUDE.md"] {
        let p: PathBuf = ctx.paths.home.join(rel);
        if p.is_dir() {
            scan_dir_for_stale(&p, &mut stale);
        } else if p.is_file() {
            if let Ok(text) = fs::read_to_string(&p) {
                let low = text.to_lowercase();
                for s in STALE {
                    if low.contains(&s.to_lowercase()) {
                        stale.push(format!("{}: {s}", p.display()));
                    }
                }
            }
        }
    }
    stale.sort();
    stale.dedup();

    // Hooks
    let settings = ctx.paths.home.join(".claude/settings.json");
    let hooks_present = fs::read_to_string(&settings)
        .ok()
        .map(|s| s.contains("sarg hook claude"))
        .unwrap_or(false);
    if hooks_present {
        ok.push("Claude hooks installed".into());
    } else {
        probs.push("Claude hooks not installed — sarg hook install (asks before flashing)".into());
    }

    if ctx.json() {
        crate::output::json(&json!({
            "ok": ok, "problems": probs, "stale_pointers": stale, "server": server_line,
            "env": ctx.resolved.env, "url": ctx.resolved.url,
        }));
        return Ok(());
    }

    for line in &ok {
        println!("{} {line}", good("✓"));
    }
    for line in &probs {
        println!("{} {line}", warn("!"));
    }
    if stale.is_empty() {
        println!("{} no stale sargineer pointers in ~/.claude", good("✓"));
    } else {
        println!("{} stale pointers (old local server or 'warehouse'):", warn("!"));
        for s in &stale {
            println!("    {}", render::dim(s));
        }
        println!("    {}", render::dim("these can make an agent skip sargineer.com or use the wrong word"));
    }
    Ok(())
}
