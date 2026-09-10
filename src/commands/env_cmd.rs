//! `sarg env`: which server sarg talks to. `prod` is the top-level
//! url/token in config.toml; anything else is a named entry under
//! `[envs.<name>]` with its own token, cache and spool. Switching is a
//! config change, so hooks and every later call follow it.

use serde_json::json;

use super::Ctx;
use crate::cli::EnvArgs;
use crate::config::{mask_token, DEFAULT_URL, PROD};
use crate::error::{Result, SargError};
use crate::output;
use crate::render;

pub fn run(ctx: &mut Ctx, args: &EnvArgs) -> Result<()> {
    let Some(name) = args
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    else {
        return show(ctx);
    };
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(SargError::usage("env names are letters, digits, - and _"));
    }

    if args.rm {
        if name == PROD {
            return Err(SargError::usage(
                "prod is the config's own url/token; sarg config unset url",
            ));
        }
        if ctx.cfg.envs.remove(name).is_none() {
            return Err(SargError::usage(format!("no env `{name}`")));
        }
        let was_active = ctx.cfg.env.as_deref() == Some(name);
        if was_active {
            ctx.cfg.env = None;
        }
        ctx.cfg.save(&ctx.paths)?;
        if ctx.json() {
            output::json(&json!({"ok": true, "removed": name, "active": PROD}));
        } else {
            println!(
                "{} removed {name}{}",
                render::good("✓"),
                if was_active { " · back on prod" } else { "" }
            );
        }
        return Ok(());
    }

    let mut changed: Vec<&str> = Vec::new();
    if name == PROD {
        if let Some(u) = &args.url {
            ctx.cfg.url = Some(u.trim_end_matches('/').to_string());
            changed.push("url");
        }
        if let Some(t) = &args.token {
            ctx.cfg.token = Some(t.trim().to_string());
            changed.push("token");
        }
    } else {
        let exists = ctx.cfg.envs.contains_key(name);
        if !exists && args.url.is_none() {
            return Err(SargError::usage(format!(
                "no env `{name}` yet — sarg env {name} --url URL [--token TOKEN] defines it"
            )));
        }
        let e = ctx.cfg.envs.entry(name.to_string()).or_default();
        if let Some(u) = &args.url {
            e.url = Some(u.trim_end_matches('/').to_string());
            changed.push("url");
        }
        if let Some(t) = &args.token {
            e.token = Some(t.trim().to_string());
            changed.push("token");
        }
    }

    let switched = if args.no_switch {
        false
    } else {
        let before = ctx.cfg.env.clone();
        ctx.cfg.env = if name == PROD {
            None
        } else {
            Some(name.to_string())
        };
        before != ctx.cfg.env
    };
    ctx.cfg.save(&ctx.paths)?;

    let url = url_of(ctx, name);
    if ctx.json() {
        output::json(&json!({
            "ok": true, "env": name, "url": url, "switched": switched,
            "changed": changed, "active": active(ctx),
        }));
        return Ok(());
    }
    let mut bits = Vec::new();
    if !changed.is_empty() {
        bits.push(format!("set {}", changed.join(" + ")));
    }
    if switched {
        bits.push("now active".into());
    } else if active(ctx) == name {
        bits.push("active".into());
    }
    println!(
        "{} {} · {}{}",
        render::good("✓"),
        render::bold(name),
        url,
        if bits.is_empty() {
            String::new()
        } else {
            render::dim(&format!(" · {}", bits.join(" · ")))
        }
    );
    if name != PROD && !args.no_switch {
        println!(
            "{}",
            render::dim(
                "every sarg call (hooks included) now goes there; sarg env prod switches back"
            )
        );
    }
    Ok(())
}

fn active(ctx: &Ctx) -> String {
    ctx.cfg.env.clone().unwrap_or_else(|| PROD.to_string())
}

fn url_of(ctx: &Ctx, name: &str) -> String {
    if name == PROD {
        ctx.cfg
            .url
            .clone()
            .unwrap_or_else(|| DEFAULT_URL.to_string())
    } else {
        ctx.cfg
            .envs
            .get(name)
            .and_then(|e| e.url.clone())
            .unwrap_or_else(|| "(no url)".into())
    }
}

fn show(ctx: &mut Ctx) -> Result<()> {
    let act = active(ctx);
    let mut rows: Vec<(String, String, bool, String)> = Vec::new();
    rows.push((
        PROD.into(),
        url_of(ctx, PROD),
        ctx.cfg
            .token
            .as_deref()
            .map(|t| !t.is_empty())
            .unwrap_or(false),
        mask_token(ctx.cfg.token.as_deref()),
    ));
    for (name, e) in &ctx.cfg.envs {
        rows.push((
            name.clone(),
            url_of(ctx, name),
            e.token.as_deref().map(|t| !t.is_empty()).unwrap_or(false),
            mask_token(e.token.as_deref()),
        ));
    }
    if ctx.json() {
        output::json(&json!({
            "active": act,
            "in_use": ctx.resolved.env,
            "envs": rows.iter().map(|(n, u, has, masked)| json!({
                "name": n, "url": u, "token": has, "token_masked": masked,
            })).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    for (name, url, has_token, masked) in &rows {
        let mark = if *name == act {
            render::good("*")
        } else {
            " ".into()
        };
        println!(
            "{mark} {:<10} {:<36} {}",
            render::bold(name),
            url,
            if *has_token {
                render::dim(&format!("token {masked}"))
            } else {
                render::warn("no token")
            }
        );
    }
    if ctx.resolved.env != act {
        println!(
            "{}",
            render::dim(&format!("(this call used --env {})", ctx.resolved.env))
        );
    }
    println!(
        "{}",
        render::dim("sarg env <name> switches · sarg env <name> --url URL --token TOKEN defines · --rm removes")
    );
    Ok(())
}
