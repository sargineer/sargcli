use super::Ctx;
use crate::cli::ConfigCmd;
use crate::config::mask_token;
use crate::error::{Result, SargError};
use crate::output;

pub fn run(ctx: &mut Ctx, cmd: &ConfigCmd) -> Result<()> {
    match cmd {
        ConfigCmd::Path => {
            println!("{}", ctx.paths.config_file.display());
            Ok(())
        }
        ConfigCmd::Get { key, reveal } => get(ctx, key.as_deref(), *reveal),
        ConfigCmd::Set { key, value } => {
            ctx.cfg.set(key, value)?;
            ctx.cfg.save(&ctx.paths)?;
            if key == "token" {
                eprintln!("token saved to {} (mode 600)", ctx.paths.config_file.display());
            } else {
                eprintln!("{key} set");
            }
            Ok(())
        }
        ConfigCmd::Unset { key } => {
            ctx.cfg.unset(key)?;
            ctx.cfg.save(&ctx.paths)?;
            eprintln!("{key} removed");
            Ok(())
        }
    }
}

fn get(ctx: &Ctx, key: Option<&str>, reveal: bool) -> Result<()> {
    let mut v = ctx.cfg.masked_json();
    if reveal {
        if let Some(o) = v.as_object_mut() {
            o.insert(
                "token".into(),
                serde_json::Value::String(ctx.cfg.token.clone().unwrap_or_default()),
            );
        }
    }
    match key {
        None => {
            if ctx.json() {
                output::json(&v);
            } else {
                output::kv("file", &ctx.paths.config_file.display().to_string());
                output::kv("url", ctx.cfg.url.as_deref().unwrap_or("(default)"));
                let tok = if reveal {
                    ctx.cfg.token.clone().unwrap_or_else(|| "(unset)".into())
                } else {
                    mask_token(ctx.cfg.token.as_deref())
                };
                output::kv("token", &tok);
                output::kv("host_tags", &ctx.cfg.host_tags.join(", "));
                output::kv("deny_terms", &ctx.cfg.deny_terms.join(", "));
                output::kv(
                    "default_project",
                    ctx.cfg.default_project.as_deref().unwrap_or(""),
                );
                for (k, a) in &ctx.cfg.project_aliases {
                    output::kv(&format!("alias.{k}"), a);
                }
                output::kv(
                    "server_seen",
                    ctx.cfg.last_seen_version.as_deref().unwrap_or(""),
                );
                output::kv(
                    "resolved",
                    &format!("{} · token from {}", ctx.resolved.url, ctx.resolved.token_source),
                );
            }
            Ok(())
        }
        Some(k) => {
            let (head, rest) = k.split_once('.').unwrap_or((k, ""));
            let mut cur = v.get(head).cloned();
            if !rest.is_empty() {
                cur = cur.and_then(|c| c.get(rest).cloned());
            }
            let cur = cur.ok_or_else(|| SargError::usage(format!("unknown config key `{k}`")))?;
            if ctx.json() {
                output::json(&cur);
            } else {
                match cur {
                    serde_json::Value::String(s) => println!("{s}"),
                    serde_json::Value::Array(a) => println!(
                        "{}",
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    serde_json::Value::Null => println!(),
                    other => println!("{other}"),
                }
            }
            Ok(())
        }
    }
}
