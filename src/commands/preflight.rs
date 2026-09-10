//! `sarg preflight`: what sarg knows about each board and each risky intent,
//! printed live and never saved — a PREFLIGHT.md read a month later is what
//! let the 09-02 flash skip the fresh query.

use serde_json::json;

use super::ask;
use super::Ctx;
use crate::cli::PreflightArgs;
use crate::error::{Result, SargError};
use crate::journal;
use crate::manifest;
use crate::output;
use crate::render::{self, note_compact, part_line};

pub fn run(ctx: &mut Ctx, args: &PreflightArgs) -> Result<()> {
    let mut sections = Vec::new();
    let mut gaps: Vec<String> = Vec::new();

    // No boards given: the project's manifest supplies them.
    let project = if args.boards.is_empty() {
        manifest::discover()
    } else {
        None
    };
    let boards: Vec<String> = if args.boards.is_empty() {
        project
            .as_ref()
            .map(|p| p.manifest.boards.clone())
            .unwrap_or_default()
    } else {
        args.boards.clone()
    };
    if boards.is_empty() && args.intents.is_empty() {
        return Err(SargError::usage(
            "name a board, or run inside a project whose sarg.yaml lists boards (sarg project add --board <product>)",
        ));
    }

    let queries: Vec<(&str, String)> = boards
        .iter()
        .map(|b| ("board", b.clone()))
        .chain(args.intents.iter().map(|i| ("intent", i.clone())))
        .collect();

    for (kind, q) in &queries {
        let m = ask::search(ctx, q, None, args.n, false, &[])?;
        if m.notes.is_empty() {
            gaps.push(q.clone());
        }
        sections.push((kind.to_string(), q.clone(), m));
    }
    journal::note("boards", json!(boards));
    if let Some(p) = &project {
        journal::note("project", p.manifest.project.clone());
    }
    journal::note("gaps", json!(gaps));

    if ctx.json() {
        output::json(&json!({
            "generated": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "live": true,
            "sections": sections.iter().map(|(k, q, m)| json!({
                "kind": k, "query": q, "total": m.total,
                "parts": m.parts, "notes": m.notes,
            })).collect::<Vec<_>>(),
            "gaps": gaps,
            "project": project.as_ref().map(|p| p.manifest.project.clone()),
        }));
        return Ok(());
    }

    let w = render::width();
    println!(
        "{} {} {}{}",
        render::bold("preflight"),
        render::dim(&chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()),
        render::dim("· live from sargineer — re-run rather than save this"),
        project
            .as_ref()
            .map(|p| render::dim(&format!(" · project {}", p.manifest.name)))
            .unwrap_or_default()
    );
    for (kind, q, m) in &sections {
        println!();
        println!(
            "{} {}{}",
            render::dim(kind),
            render::bold(q),
            render::dim(&format!(" · {} of {} lessons", m.notes.len(), m.total))
        );
        for p in m.parts.iter().take(3) {
            println!("  part  {}", part_line(p));
        }
        if m.notes.is_empty() {
            println!(
                "  {}",
                render::warn("no lessons — nobody has paid for this yet")
            );
        }
        for (i, n) in m.notes.iter().enumerate() {
            print!("{}", note_compact(i + 1, n, w));
        }
    }
    println!();
    if gaps.is_empty() {
        println!(
            "{}",
            render::good("gaps: none — every board and intent has at least one lesson")
        );
    } else {
        println!(
            "{}",
            render::warn(&format!(
                "gaps: {} — tell the user before the evening goes; record what you learn with sarg lesson new",
                gaps.iter().map(|g| format!("\"{g}\"")).collect::<Vec<_>>().join(", ")
            ))
        );
    }
    Ok(())
}
