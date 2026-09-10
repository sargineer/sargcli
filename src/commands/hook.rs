//! `sarg guard`/`sarg hint` (agent-neutral, for shell wrappers and any
//! agent's hooks), `sarg hook claude <event>` (the Claude adapter), and
//! installing/removing sarg's hooks in Claude Code's settings.

use std::fs;

use serde_json::{json, Value};

use super::Ctx;
use crate::cli::{ClaudeEvent, HookCmd};
use crate::error::{Result, SargError};
use crate::hooks::{claude, core};
use crate::journal;
use crate::render;

/// `sarg guard <command...>` — exit 0 to allow, 2 to block (with hits on
/// stderr). Shell wrappers do: `sarg guard "$@" || { echo "search first"; exit 1; }`.
pub fn guard(ctx: &mut Ctx, command: &[String]) -> Result<i32> {
    let cmd = command.join(" ");
    let session = crate::agent::detect().session_id().unwrap_or_else(|| "-".into());
    let d = core::guard(ctx, &session, &cmd);
    journal::note("blocked", d.block);
    journal::note("guard_cmd", cmd);
    if ctx.json() {
        crate::output::json(&json!({
            "block": d.block, "reason": d.reason, "query": d.query,
        }));
        return Ok(if d.block { 2 } else { 0 });
    }
    if d.block {
        eprintln!("{}", d.reason);
        if !d.hits.is_empty() {
            eprint!("{}", d.hits);
        }
        Ok(2)
    } else {
        Ok(0)
    }
}

/// `sarg hint <text...>` — prints one line or nothing.
pub fn hint(_ctx: &mut Ctx, text: &[String]) -> Result<i32> {
    if let Some(line) = core::hint(&text.join(" ")) {
        println!("{line}");
    }
    Ok(0)
}

pub fn run(ctx: &mut Ctx, cmd: &HookCmd) -> Result<i32> {
    match cmd {
        HookCmd::Claude { event } => claude::run(ctx, *event),
        HookCmd::Install { dry_run } => install(ctx, *dry_run),
        HookCmd::Uninstall => uninstall(ctx),
        HookCmd::Status => status(ctx),
    }
}

const EVENTS: [(&str, Option<&str>, ClaudeEvent); 3] = [
    ("PreToolUse", Some("Bash"), ClaudeEvent::Pretooluse),
    ("UserPromptSubmit", None, ClaudeEvent::Userpromptsubmit),
    ("Stop", None, ClaudeEvent::Stop),
];

fn event_cmd(event: ClaudeEvent) -> &'static str {
    match event {
        ClaudeEvent::Pretooluse => "sarg hook claude pretooluse",
        ClaudeEvent::Userpromptsubmit => "sarg hook claude userpromptsubmit",
        ClaudeEvent::Stop => "sarg hook claude stop",
    }
}

fn settings_path(ctx: &Ctx) -> std::path::PathBuf {
    ctx.paths.home.join(".claude/settings.json")
}

fn has_hook(settings: &Value, event_name: &str, command: &str) -> bool {
    settings
        .get("hooks")
        .and_then(|h| h.get(event_name))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter().any(|entry| {
                entry
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hs| hs.iter().any(|h| h.get("command").and_then(|c| c.as_str()) == Some(command)))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn install(ctx: &mut Ctx, dry_run: bool) -> Result<i32> {
    let path = settings_path(ctx);
    let mut settings: Value = match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| {
            SargError::usage(format!("{} is not valid JSON: {e}", path.display()))
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.into()),
    };
    if !settings.is_object() {
        return Err(SargError::usage(format!("{} is not a JSON object", path.display())));
    }
    let mut added = Vec::new();
    for (event_name, matcher, event) in EVENTS {
        let command = event_cmd(event);
        if has_hook(&settings, event_name, command) {
            continue;
        }
        let entry = match matcher {
            Some(m) => json!({"matcher": m, "hooks": [{"type":"command","command":command}]}),
            None => json!({"hooks": [{"type":"command","command":command}]}),
        };
        let hooks = settings
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| json!({}));
        let arr = hooks
            .as_object_mut()
            .ok_or_else(|| SargError::usage("settings.hooks is not an object"))?
            .entry(event_name.to_string())
            .or_insert_with(|| json!([]));
        arr.as_array_mut()
            .ok_or_else(|| SargError::usage(format!("settings.hooks.{event_name} is not a list")))?
            .push(entry);
        added.push(format!("{event_name} → {command}"));
    }
    if dry_run {
        if added.is_empty() {
            println!("all sarg hooks already present in {}", path.display());
        } else {
            println!("would add to {}:", path.display());
            for a in &added {
                println!("  {a}");
            }
        }
        return Ok(0);
    }
    if added.is_empty() {
        println!("{} already installed in {}", render::good("✓"), path.display());
        return Ok(0);
    }
    // Back up before touching the user's settings.
    if path.exists() {
        let backup = path.with_extension("json.sarg-bak");
        fs::copy(&path, &backup)?;
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&settings)?)?;
    println!("{} installed into {}:", render::good("✓"), path.display());
    for a in &added {
        println!("  {a}");
    }
    println!("{}", render::dim("start a new Claude Code session for them to take effect · sarg hook uninstall to remove"));
    Ok(0)
}

fn uninstall(ctx: &mut Ctx) -> Result<i32> {
    let path = settings_path(ctx);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            println!("no settings file at {}", path.display());
            return Ok(0);
        }
    };
    let mut settings: Value = serde_json::from_str(&text)?;
    let mut removed = 0;
    if let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for (_event, entries) in hooks.iter_mut() {
            if let Some(arr) = entries.as_array_mut() {
                let before = arr.len();
                arr.retain(|entry| {
                    !entry
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|hs| {
                            hs.iter().any(|h| {
                                h.get("command")
                                    .and_then(|c| c.as_str())
                                    .map(|c| c.starts_with("sarg hook claude"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                });
                removed += before - arr.len();
            }
        }
    }
    fs::write(&path, serde_json::to_string_pretty(&settings)?)?;
    println!("{} removed {removed} sarg hook(s) from {}", render::good("✓"), path.display());
    Ok(0)
}

fn status(ctx: &mut Ctx) -> Result<i32> {
    let path = settings_path(ctx);
    let settings: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(json!({}));
    let rows: Vec<(String, bool)> = EVENTS
        .iter()
        .map(|(name, _, ev)| {
            let cmd = event_cmd(*ev);
            (format!("{name}: {cmd}"), has_hook(&settings, name, cmd))
        })
        .collect();
    let all = rows.iter().all(|(_, ok)| *ok);
    if ctx.json() {
        crate::output::json(&json!({
            "settings": path,
            "installed": all,
            "hooks": rows.iter().map(|(n, ok)| json!({"hook": n, "installed": ok})).collect::<Vec<_>>(),
        }));
        return Ok(0);
    }
    println!("Claude Code hooks in {}:", path.display());
    for (name, ok) in &rows {
        println!("  {} {name}", if *ok { render::good("✓") } else { render::dim("·") });
    }
    if !all {
        println!("{}", render::dim("sarg hook install to add the missing ones"));
    }
    Ok(0)
}
