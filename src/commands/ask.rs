//! `sarg ask`: parts and lessons in one shot. Fans out to /search (parts +
//! note ids) and /notes?full=1 (answers inline) in parallel, merges, ranks
//! by status then hardware match, and prints compact hits.

use serde_json::{json, Value};

use super::Ctx;
use crate::cli::AskArgs;
use crate::client::urlencode;
use crate::error::{Result, SargError};
use crate::journal;
use crate::manifest;
use crate::output;
use crate::render::{self, list, note_compact, note_full, part_line, s, status_rank, u};

pub struct Merged {
    pub query: String,
    pub parts: Vec<Value>,
    pub notes: Vec<Value>,
    pub total: u64,
}

/// The two calls, merged and ranked. Shared with `preflight`. `boost`
/// names product ids (the project's boards) whose lessons rank first.
pub fn search(
    ctx: &Ctx,
    q: &str,
    hw: Option<&str>,
    n: usize,
    mine: bool,
    boost: &[String],
) -> Result<Merged> {
    let mut notes_path = format!("/notes?q={}&full=1&n={}", urlencode(q), n.max(1));
    if let Some(h) = hw {
        notes_path.push_str(&format!("&hw={}", urlencode(h)));
    }
    if mine {
        notes_path.push_str("&mine=1");
    }
    let search_path = format!("/search?q={}", urlencode(q));
    let client = &ctx.client;

    let (search, notes) = std::thread::scope(|sc| {
        let a = sc.spawn(move || client.get_json(&search_path));
        let b = sc.spawn(move || client.get_json(&notes_path));
        (a.join(), b.join())
    });
    let notes =
        notes.map_err(|_| SargError::other(anyhow::anyhow!("search thread panicked")))??;
    // /search is a bonus (parts + extra note ids); a failure there must not hide lessons.
    let search = match search {
        Ok(Ok(v)) => v,
        _ => Value::Null,
    };

    let parts: Vec<Value> = search
        .get("parts")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let total = u(&notes, "total");
    let mut hits: Vec<Value> = notes
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    // Note ids /search knew about that /notes did not return: keep them as title-only hits.
    let known: std::collections::HashSet<String> = hits.iter().map(render::note_ref).collect();
    if let Some(extra) = search.get("notes").and_then(|n| n.as_array()) {
        for e in extra {
            if !known.contains(&render::note_ref(e)) && hits.len() < n {
                let mut e = e.clone();
                if let Some(o) = e.as_object_mut() {
                    o.insert("title_only".into(), json!(true));
                }
                hits.push(e);
            }
        }
    }

    // Rank: status ladder first, then hardware match, then the server's order.
    let mut part_ids: Vec<String> = parts
        .iter()
        .map(|p| s(p, "product").to_lowercase())
        .collect();
    part_ids.extend(boost.iter().map(|b| b.to_lowercase()));
    let q_words: Vec<String> = q
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 3)
        .collect();
    let hw_score = |v: &Value| -> u8 {
        let hw: Vec<String> = list(v, "hw").iter().map(|h| h.to_lowercase()).collect();
        let hits_part = hw
            .iter()
            .any(|h| part_ids.iter().any(|p| p.contains(h) || h.contains(p)));
        let hits_word = hw
            .iter()
            .any(|h| q_words.iter().any(|w| h.contains(w.as_str())));
        u8::from(hits_part) + u8::from(hits_word)
    };
    let mut keyed: Vec<(u8, u8, usize, Value)> = hits
        .into_iter()
        .enumerate()
        .map(|(i, v)| (status_rank(s(&v, "status")), 2 - hw_score(&v), i, v))
        .collect();
    keyed.sort_by_key(|k| (k.0, k.1, k.2));
    let notes = keyed.into_iter().map(|k| k.3).collect();

    Ok(Merged {
        query: q.to_string(),
        parts,
        notes,
        total,
    })
}

pub fn run(ctx: &mut Ctx, args: &AskArgs) -> Result<()> {
    let q = args.query.join(" ").trim().to_string();
    if q.is_empty() && args.hw.is_none() {
        return Err(SargError::usage(
            "give me words to search, or --hw <product>",
        ));
    }
    // Inside a project, its boards rank first and are named in the header.
    let project = if args.hw.is_none() {
        manifest::discover()
    } else {
        None
    };
    let boost: Vec<String> = project
        .as_ref()
        .map(|p| p.manifest.boards.clone())
        .unwrap_or_default();
    let m = search(ctx, &q, args.hw.as_deref(), args.n, args.mine, &boost)?;
    journal::note("q", q.clone());
    if let Some(p) = &project {
        journal::note("project", p.manifest.project.clone());
    }
    journal::note("hits", m.notes.len() as u64);

    if ctx.json() {
        output::json(&json!({
            "query": m.query,
            "total": m.total,
            "parts": m.parts,
            "notes": m.notes,
            "signed_in": ctx.client.has_token(),
            "project": project.as_ref().map(|p| json!({
                "name": p.manifest.name, "project": p.manifest.project, "boards": p.manifest.boards,
            })),
        }));
        return Ok(());
    }

    let w = render::width();
    if let Some(p) = &project {
        if !p.manifest.boards.is_empty() {
            println!(
                "{}",
                render::dim(&format!(
                    "project {} · boards {} rank first",
                    p.manifest.name,
                    p.manifest.boards.join(" ")
                ))
            );
        }
    }
    let title_only = m
        .notes
        .iter()
        .filter(|v| v.get("title_only").and_then(|t| t.as_bool()) == Some(true))
        .count();
    let full = m.notes.len() - title_only;
    println!(
        "{} {} · {} lesson{}{} · {} match in all · {} part{}{}",
        render::bold("sarg"),
        render::dim(&format!("\"{}\"", m.query)),
        full,
        if full == 1 { "" } else { "s" },
        if title_only > 0 {
            format!(" + {title_only} title-only")
        } else {
            String::new()
        },
        m.total,
        m.parts.len(),
        if m.parts.len() == 1 { "" } else { "s" },
        if ctx.client.has_token() {
            String::new()
        } else {
            render::warn(" · signed out: fixes and steps are locked")
        }
    );
    if !m.parts.is_empty() {
        for p in m.parts.iter().take(5) {
            println!("  part  {}", part_line(p));
        }
    }
    if m.notes.is_empty() {
        println!();
        println!(
            "{}",
            render::warn(&format!(
                "no lessons about \"{}\" — nobody has paid for this one yet; worth saying so before the evening goes",
                m.query
            ))
        );
        return Ok(());
    }
    println!();
    for (i, v) in m.notes.iter().enumerate() {
        if args.verbose {
            print!("{}", note_full(v, w));
            println!("{}", render::dim(&"─".repeat(w.min(80))));
        } else {
            print!("{}", note_compact(i + 1, v, w));
        }
    }
    if !args.verbose {
        let first = m.notes.first().map(render::note_ref).unwrap_or_default();
        println!();
        println!(
            "{}",
            render::dim(&format!(
                "next: sarg show {first} · sarg ask -v {} · sarg ask -n {} {}",
                shell_words(&m.query),
                args.n * 3,
                shell_words(&m.query)
            ))
        );
    }
    Ok(())
}

fn shell_words(q: &str) -> String {
    if q.chars()
        .any(|c| c.is_whitespace() || "'\"$`\\".contains(c))
    {
        format!("'{}'", q.replace('\'', "'\\''"))
    } else {
        q.to_string()
    }
}
