//! `sarg stats`: what sarg has been asked, from the local journal. Answers
//! "how often did we ask before flashing" without mining transcripts.

use std::collections::BTreeMap;
use std::fs;

use serde_json::{json, Value};

use super::Ctx;
use crate::error::Result;
use crate::render::{self, s};

pub fn run(ctx: &mut Ctx, session: Option<&str>) -> Result<()> {
    let path = ctx.paths.sargineer_dir.join("journal.jsonl");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut by_verb: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_agent: BTreeMap<String, u64> = BTreeMap::new();
    let mut sessions: std::collections::BTreeSet<String> = Default::default();
    let mut total = 0u64;
    let mut guard_blocks = 0u64;
    let mut guard_allows = 0u64;
    let mut lessons = 0u64;

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(want) = session {
            if s(&v, "session") != want {
                continue;
            }
        }
        total += 1;
        *by_verb.entry(s(&v, "verb").to_string()).or_default() += 1;
        *by_agent.entry(s(&v, "agent").to_string()).or_default() += 1;
        let sess = s(&v, "session");
        if !sess.is_empty() && sess != "-" {
            sessions.insert(sess.to_string());
        }
        if v.get("guard_cmd").is_some() {
            if v.get("blocked").and_then(|b| b.as_bool()) == Some(true) {
                guard_blocks += 1;
            } else {
                guard_allows += 1;
            }
        }
        if s(&v, "verb").starts_with("lesson.new")
            || s(&v, "verb").starts_with("issue")
            || s(&v, "verb").starts_with("feedback")
        {
            lessons += 1;
        }
    }

    if ctx.json() {
        crate::output::json(&json!({
            "total": total, "sessions": sessions.len(),
            "guard_blocks": guard_blocks, "guard_allows": guard_allows,
            "notes_written": lessons,
            "by_verb": by_verb, "by_agent": by_agent,
        }));
        return Ok(());
    }

    if total == 0 {
        println!("journal is empty ({})", path.display());
        return Ok(());
    }
    println!(
        "{} {total} calls · {} sessions · {} notes written{}",
        render::bold("sarg stats"),
        sessions.len(),
        lessons,
        session.map(|s| format!(" · session {s}")).unwrap_or_default()
    );
    if guard_blocks + guard_allows > 0 {
        println!(
            "guard: {} flash command(s) held for a search, {} passed",
            guard_blocks, guard_allows
        );
    }
    println!("by agent:");
    for (a, n) in &by_agent {
        println!("  {a:<14} {n}");
    }
    println!("top verbs:");
    let mut verbs: Vec<_> = by_verb.iter().collect();
    verbs.sort_by(|a, b| b.1.cmp(a.1));
    for (verb, n) in verbs.into_iter().take(12) {
        println!("  {verb:<22} {n}");
    }
    Ok(())
}
