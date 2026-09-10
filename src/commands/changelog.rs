use serde_json::Value;

use super::Ctx;
use crate::error::Result;
use crate::output;

pub fn run(ctx: &mut Ctx, since: Option<&str>, all: bool) -> Result<()> {
    let since = if all {
        None
    } else {
        since
            .map(str::to_string)
            .or_else(|| ctx.cfg.previous_version_for(&ctx.resolved.env))
    };
    let path = match &since {
        Some(s) => format!("/changelog?since={}", urlencode(s)),
        None => "/changelog".to_string(),
    };
    let headers = [("Accept".to_string(), "application/json".to_string())];
    let r = ctx.client.ok(ctx.client.send("GET", &path, None, &headers)?)?;
    let v = r.json().unwrap_or(Value::Null);
    if ctx.json() {
        output::json(&v);
        return Ok(());
    }
    let entries = v.get("entries").and_then(|e| e.as_array());
    match entries {
        Some(es) if es.is_empty() => {
            println!(
                "nothing new{} · server {}",
                since.map(|s| format!(" since {s}")).unwrap_or_default(),
                v.get("version").and_then(|x| x.as_str()).unwrap_or("?")
            );
        }
        Some(es) => {
            for e in es {
                if let Some(md) = e.get("md").and_then(|m| m.as_str()) {
                    println!("{}", md.trim_end());
                    println!();
                } else {
                    println!("{e}");
                }
            }
        }
        None => println!("{}", r.body.trim_end()),
    }
    Ok(())
}

use crate::client::urlencode;
