//! `sarg stock`: the local on-hand file, minimal. `ls` reads it, `set`
//! writes one line, `rm` drops one. `sarg bom check` is the consumer.

use serde_json::json;

use super::Ctx;
use crate::cli::StockCmd;
use crate::error::{Result, SargError};
use crate::manifest::slugify;
use crate::output;
use crate::render;
use crate::stock::{self, Item};

pub fn run(ctx: &mut Ctx, cmd: &StockCmd) -> Result<()> {
    match cmd {
        StockCmd::Ls => ls(ctx),
        StockCmd::Set {
            id,
            qty,
            location,
            note,
        } => set(ctx, id, *qty, location.as_deref(), note.as_deref()),
        StockCmd::Rm { id } => rm(ctx, id),
    }
}

fn key(id: &str) -> String {
    let id = id.trim().trim_start_matches("/p/");
    if id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        id.to_string()
    } else {
        slugify(id)
    }
}

fn ls(ctx: &mut Ctx) -> Result<()> {
    let st = stock::load(&ctx.paths)?;
    if ctx.json() {
        output::json(&st);
        return Ok(());
    }
    if st.is_empty() {
        println!(
            "{}",
            render::dim(&format!(
                "nothing tracked — sarg stock set <product-id> <qty> [--where BIN] · {}",
                stock::path(&ctx.paths).display()
            ))
        );
        return Ok(());
    }
    for (k, it) in &st {
        let mut bits = vec![format!("×{:<4}", it.qty), render::bold(k)];
        if let Some(w) = &it.location {
            bits.push(render::dim(&format!("@ {w}")));
        }
        if let Some(n) = &it.note {
            bits.push(render::dim(n));
        }
        println!("{}", bits.join(" "));
    }
    Ok(())
}

fn set(
    ctx: &mut Ctx,
    id: &str,
    qty: u32,
    location: Option<&str>,
    note: Option<&str>,
) -> Result<()> {
    let k = key(id);
    if k.is_empty() {
        return Err(SargError::usage("give a product id or an item name"));
    }
    let mut st = stock::load(&ctx.paths)?;
    let entry = st.entry(k.clone()).or_default();
    entry.qty = qty;
    if location.is_some() {
        entry.location = location.map(str::to_string);
    }
    if note.is_some() {
        entry.note = note.map(str::to_string);
    }
    entry.updated = Some(chrono::Local::now().format("%Y-%m-%d").to_string());
    let saved: Item = entry.clone();
    stock::save(&ctx.paths, &st)?;
    if ctx.json() {
        output::json(&json!({"ok": true, "id": k, "item": saved}));
        return Ok(());
    }
    println!(
        "{} {} ×{}{}",
        render::good("✓"),
        render::bold(&k),
        qty,
        saved
            .location
            .as_deref()
            .map(|w| format!(" @ {w}"))
            .unwrap_or_default()
    );
    Ok(())
}

fn rm(ctx: &mut Ctx, id: &str) -> Result<()> {
    let k = key(id);
    let mut st = stock::load(&ctx.paths)?;
    let removed = st.remove(&k).is_some();
    stock::save(&ctx.paths, &st)?;
    if ctx.json() {
        output::json(&json!({"ok": true, "id": k, "removed": removed}));
        return Ok(());
    }
    if removed {
        println!("{} removed {k}", render::good("✓"));
    } else {
        println!("{}", render::dim(&format!("{k} was not tracked")));
    }
    Ok(())
}
