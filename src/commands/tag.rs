//! `sarg tag <t>` and `sarg vendor <v>`: the same shape, different path.

use super::Ctx;
use crate::error::Result;
use crate::output;
use crate::render::{self, part_line, s, status_badge, truncate};

#[derive(Clone, Copy)]
pub enum Kind {
    Tag,
    Vendor,
}

pub fn run(ctx: &mut Ctx, kind: Kind, name: &str, n: usize) -> Result<()> {
    let name = name.trim().trim_start_matches('/');
    let path = match kind {
        Kind::Tag => format!("/t/{name}"),
        Kind::Vendor => format!("/v/{name}"),
    };
    let v = ctx.client.get_json(&path)?;
    if ctx.json() {
        output::json(&v);
        return Ok(());
    }
    let w = render::width();
    let parts = v.get("parts").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let notes = v.get("notes").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    println!(
        "{} {} · {} part{} · {} lesson{}",
        render::bold(match kind {
            Kind::Tag => "tag",
            Kind::Vendor => "vendor",
        }),
        render::bold(name),
        parts.len(),
        if parts.len() == 1 { "" } else { "s" },
        notes.len(),
        if notes.len() == 1 { "" } else { "s" },
    );
    for p in parts.iter().take(n) {
        println!("  part  {}", part_line(p));
    }
    if parts.len() > n {
        println!("{}", render::dim(&format!("  … {} more parts", parts.len() - n)));
    }
    if !notes.is_empty() {
        println!();
    }
    for nt in notes.iter().take(n) {
        println!(
            "  {} {} {}",
            status_badge(s(nt, "status")),
            render::bold(&format!("{}/{}", s(nt, "handle"), s(nt, "id"))),
            render::dim(&truncate(s(nt, "title"), w.saturating_sub(30)))
        );
    }
    if notes.len() > n {
        println!(
            "{}",
            render::dim(&format!("  … {} more · -n {} to see them", notes.len() - n, notes.len()))
        );
    }
    Ok(())
}
