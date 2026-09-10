//! `sarg bom check`: on hand vs to buy, from the manifest's BOM. On-hand
//! comes from the local stock file first, then from what sargineer says
//! you own; buy links come from the part page. Free-text lines cannot be
//! looked up and are reported as untracked until `sarg stock set` names
//! them.

use std::path::Path;

use serde_json::{json, Value};

use super::Ctx;
use crate::cli::BomCmd;
use crate::error::Result;
use crate::manifest::{self, BomLine};
use crate::output;
use crate::render::{self, list, s, u};
use crate::stock;

pub fn run(ctx: &mut Ctx, cmd: &BomCmd) -> Result<()> {
    match cmd {
        BomCmd::Check { dir, offline } => check(ctx, dir.as_deref(), *offline),
    }
}

#[derive(Debug)]
struct Row {
    line: BomLine,
    have: Option<u32>,
    source: &'static str,
    buy: String,
    name: String,
}

impl Row {
    fn short(&self) -> u32 {
        self.line.qty.saturating_sub(self.have.unwrap_or(0))
    }
    fn state(&self) -> &'static str {
        match self.have {
            None => "untracked",
            Some(h) if h >= self.line.qty => "on-hand",
            Some(_) => "buy",
        }
    }
}

fn part_info(ctx: &Ctx, id: &str) -> Option<(u32, String, String)> {
    let v = ctx.client.get_json(&format!("/p/{id}")).ok()?;
    let part = v.get("part").cloned().unwrap_or(Value::Null);
    let facts = v.get("facts").cloned().unwrap_or(Value::Null);
    let owned = u(&part, "owned_qty").max(u(&facts, "qty_owned")) as u32;
    let buy = list(&part, "buy")
        .into_iter()
        .next()
        .unwrap_or_else(|| s(&part, "url").to_string());
    let name = if s(&part, "name").is_empty() {
        s(&facts, "name").to_string()
    } else {
        s(&part, "name").to_string()
    };
    Some((owned, buy, name))
}

fn check(ctx: &mut Ctx, dir: Option<&Path>, offline: bool) -> Result<()> {
    let found = manifest::find(dir)?;
    let local = stock::load(&ctx.paths)?;
    let mut rows: Vec<Row> = Vec::new();
    for line in &found.manifest.bom {
        let key = line.key();
        let mut row = Row {
            line: line.clone(),
            have: None,
            source: "",
            buy: String::new(),
            name: String::new(),
        };
        if let Some(item) = local.get(&key) {
            row.have = Some(item.qty);
            row.source = "stock";
        }
        if let Some(id) = &line.product {
            if !offline {
                if let Some((owned, buy, name)) = part_info(ctx, id) {
                    row.buy = buy;
                    row.name = name;
                    if row.have.is_none() {
                        row.have = Some(owned);
                        row.source = "sargineer";
                    }
                }
            }
        }
        rows.push(row);
    }

    let on_hand = rows.iter().filter(|r| r.state() == "on-hand").count();
    let to_buy: Vec<&Row> = rows.iter().filter(|r| r.state() == "buy").collect();
    let untracked: Vec<&Row> = rows.iter().filter(|r| r.state() == "untracked").collect();

    if ctx.json() {
        output::json(&json!({
            "project": found.manifest.project,
            "dir": found.dir,
            "lines": rows.iter().map(|r| json!({
                "key": r.line.key(), "label": r.line.label(), "name": r.name,
                "need": r.line.qty, "have": r.have, "short": r.short(),
                "state": r.state(), "source": r.source, "buy": r.buy,
            })).collect::<Vec<_>>(),
            "on_hand": on_hand, "buy": to_buy.len(), "untracked": untracked.len(),
        }));
        return Ok(());
    }

    println!(
        "{} {} · {} line{}{}",
        render::bold("bom"),
        found.manifest.name,
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        if offline {
            render::dim(" · offline: stock file only")
        } else {
            String::new()
        }
    );
    if rows.is_empty() {
        println!(
            "{}",
            render::dim("no bom lines — sarg project add <product-id> --qty N, or --item \"M2.5x6 screws\" --qty 8")
        );
        return Ok(());
    }
    for r in &rows {
        let mark = match r.state() {
            "on-hand" => render::good("✓"),
            "buy" => render::warn("✗"),
            _ => render::dim("?"),
        };
        let have = match r.have {
            Some(h) => format!("have {h}"),
            None => "have ?".into(),
        };
        let mut tail = Vec::new();
        if !r.name.is_empty() {
            tail.push(render::dim(&render::truncate(&r.name, 40)));
        }
        if r.state() == "buy" {
            tail.push(render::warn(&format!("short {}", r.short())));
            if !r.buy.is_empty() {
                tail.push(r.buy.clone());
            }
        }
        if r.state() == "untracked" {
            tail.push(render::dim(&format!(
                "sarg stock set {} <qty> to track it",
                r.line.key()
            )));
        }
        println!(
            "  {mark} {:<28} need {:<4} {:<9} {}",
            render::truncate(&r.line.label(), 28),
            r.line.qty,
            have,
            tail.join("  ")
        );
    }
    println!();
    let mut summary = vec![format!("on hand {on_hand} of {}", rows.len())];
    if !to_buy.is_empty() {
        summary.push(render::warn(&format!(
            "buy {}: {}",
            to_buy.len(),
            to_buy
                .iter()
                .map(|r| r.line.label())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if !untracked.is_empty() {
        summary.push(render::dim(&format!("untracked {}", untracked.len())));
    }
    println!("{}", summary.join(" · "));
    Ok(())
}
