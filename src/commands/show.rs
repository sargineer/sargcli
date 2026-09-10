use serde_json::Value;

use super::Ctx;
use crate::client::urlencode;
use crate::error::{Result, SargError};
use crate::output;
use crate::render::{self, note_full, s};

/// Fetch `<handle>/<id>`; a bare id is looked up by search.
pub fn fetch(ctx: &Ctx, r#ref: &str) -> Result<Value> {
    let r = r#ref.trim().trim_start_matches("/n/").trim_start_matches('/');
    if let Some((h, id)) = r.split_once('/') {
        return ctx.client.get_json(&format!("/n/{h}/{id}"));
    }
    let found = ctx
        .client
        .get_json(&format!("/notes?q={}&n=20", urlencode(r)))?;
    let hit = found
        .get("results")
        .and_then(|a| a.as_array())
        .and_then(|a| a.iter().find(|n| s(n, "id") == r).cloned());
    match hit {
        Some(n) => ctx
            .client
            .get_json(&format!("/n/{}/{}", s(&n, "handle"), s(&n, "id"))),
        None => Err(SargError::usage(format!(
            "no note with id `{r}` — give <handle>/<id> (sarg ask finds them)"
        ))),
    }
}

pub fn run(ctx: &mut Ctx, r#ref: &str) -> Result<()> {
    let v = fetch(ctx, r#ref)?;
    if ctx.json() {
        output::json(&v);
        return Ok(());
    }
    print!("{}", note_full(&v, render::width()));
    if let Some(locked) = v.get("locked").and_then(|l| l.as_array()) {
        if !locked.is_empty() {
            println!(
                "{}",
                render::warn(&format!(
                    "locked (sign in to read): {}",
                    locked
                        .iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            );
        }
    }
    Ok(())
}
