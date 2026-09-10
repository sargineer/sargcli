use serde_json::{json, Value};

use super::Ctx;
use crate::cli::AuthCmd;
use crate::config::TokenSource;
use crate::error::{Result, SargError};
use crate::output;

pub fn run(ctx: &mut Ctx, cmd: &AuthCmd) -> Result<()> {
    match cmd {
        AuthCmd::Status => status(ctx),
        AuthCmd::LoginLink { open } => login_link(ctx, *open),
        AuthCmd::TokenMint => token_mint(ctx),
    }
}

fn require_token(ctx: &Ctx) -> Result<()> {
    if ctx.resolved.token_source == TokenSource::None {
        return Err(SargError::Auth {
            message: "no token: not signed in to sargineer".into(),
            hint: Some(
                "sarg config set token <token> (saved in ~/.sargineer/config.toml); \
                 no account yet → sarg doc start"
                    .into(),
            ),
        });
    }
    Ok(())
}

fn status(ctx: &mut Ctx) -> Result<()> {
    let public = |ctx: &Ctx| -> Result<Value> {
        let r = ctx.client.send("GET", "/", None, &[])?;
        Ok(r.json().unwrap_or(Value::Null))
    };
    if ctx.resolved.token_source == TokenSource::None {
        let p = public(ctx)?;
        if ctx.json() {
            output::json(&json!({
                "signed_in": false,
                "server": ctx.resolved.url,
                "token_source": ctx.resolved.token_source,
                "public": p,
            }));
        } else {
            println!("not signed in · {}", ctx.resolved.url);
            if let (Some(l), Some(m)) = (p.get("public_lessons"), p.get("public_models")) {
                println!("public: {l} lessons, {m} models readable without an account");
            }
            println!("sign in: sarg config set token <token> · no account yet → sarg doc start");
        }
        return Err(SargError::Auth {
            message: "no token configured".into(),
            hint: None,
        });
    }
    let me = ctx.client.get_json("/me")?;
    let agent = crate::agent::detect();
    if ctx.json() {
        let mut v = json!({
            "signed_in": true,
            "server": ctx.resolved.url,
            "token_source": ctx.resolved.token_source,
            "agent": agent.name(),
            "session": agent.session_id(),
        });
        if let (Some(dst), Some(src)) = (v.as_object_mut(), me.as_object()) {
            for (k, val) in src {
                dst.insert(k.clone(), val.clone());
            }
        }
        output::json(&v);
    } else {
        let s = |k: &str| me.get(k).and_then(|v| v.as_str()).unwrap_or("?").to_string();
        println!(
            "signed in as {} ({}) since {} · token from {} · {} · called by {}",
            s("handle"),
            s("role"),
            s("created").chars().take(10).collect::<String>(),
            ctx.resolved.token_source,
            ctx.resolved.url,
            agent.name()
        );
    }
    Ok(())
}

fn login_link(ctx: &mut Ctx, open: bool) -> Result<()> {
    require_token(ctx)?;
    let r = ctx.client.post_empty("/login/link")?;
    let v = r.json().unwrap_or(Value::Null);
    let url = v
        .get("url")
        .or_else(|| v.get("link"))
        .and_then(|u| u.as_str())
        .map(str::to_string)
        .or_else(|| {
            let t = r.body.trim();
            t.starts_with("http").then(|| t.to_string())
        })
        .ok_or_else(|| {
            SargError::other(anyhow::anyhow!("no sign-in URL in the answer: {}", r.body))
        })?;
    if ctx.json() {
        output::json(&json!({ "url": url, "valid_for": "10 minutes, one use", "raw": v }));
    } else {
        println!("{url}");
        eprintln!("valid ten minutes, works once — it signs the browser in as you");
    }
    if open {
        open::that(&url).map_err(SargError::other)?;
    }
    Ok(())
}

fn token_mint(ctx: &mut Ctx) -> Result<()> {
    require_token(ctx)?;
    let r = ctx.client.post_empty("/tokens")?;
    let v = r.json().unwrap_or(Value::Null);
    let tok = v
        .get("token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| SargError::other(anyhow::anyhow!("no token in the answer: {}", r.body)))?;
    if ctx.json() {
        output::json(&v);
    } else {
        println!("{tok}");
        eprintln!("shown once. store it: sarg config set token <token>  (the old token stays valid)");
    }
    Ok(())
}
