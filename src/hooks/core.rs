//! Agent-neutral hook logic: should a command be preceded by a search
//! (guard), what one line of context fits a prompt (hint), and is it worth
//! nudging for a lesson at the end (nudge). Each returns a decision plus
//! text; the adapters decide how to signal it to their agent.

use std::collections::BTreeSet;
use std::fs;

use serde_json::Value;

use crate::client::urlencode;
use crate::commands::Ctx;
use crate::render::{s, status_rank, truncate};

/// Tools that write to a board — the moment a stale assumption gets burned in.
const FLASH_TOOLS: [&str; 18] = [
    "esptool",
    "esptool.py",
    "idf.py",
    "arduino-cli",
    "pio",
    "platformio",
    "mpremote",
    "ampy",
    "rshell",
    "picotool",
    "dfu-util",
    "nrfutil",
    "adafruit-nrfutil",
    "uf2conv",
    "west",
    "openocd",
    "avrdude",
    "meshcore-cli",
];

/// Hardware words we recognise in a command or prompt, to aim the search.
const HW_WORDS: [&str; 24] = [
    "esp32-s3",
    "esp32-c3",
    "esp32-c6",
    "esp32-s2",
    "esp32",
    "heltec",
    "rp2040",
    "rp2350",
    "pico",
    "stm32",
    "nrf52840",
    "nrf52",
    "nrf53",
    "openmv",
    "teensy",
    "samd21",
    "sx1262",
    "sx1276",
    "meshcore",
    "meshtastic",
    "ov3660",
    "ov5640",
    "gc0308",
    "luckfox",
];

pub struct GuardDecision {
    pub block: bool,
    /// One-line reason (why blocked, or why allowed-silently is empty).
    pub reason: String,
    /// Compact hits to show, already rendered.
    pub hits: String,
    pub query: String,
}

fn tokenize(cmd: &str) -> Vec<String> {
    cmd.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '.' && c != '_')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

/// Does this command write firmware / bring up a board?
pub fn is_flash(cmd: &str) -> bool {
    let toks = tokenize(cmd);
    let tool = toks.iter().any(|t| {
        let base = t.rsplit('/').next().unwrap_or(t);
        FLASH_TOOLS.contains(&base)
    });
    if !tool {
        return false;
    }
    // idf.py / pio / west are only flashing when they say so.
    let needs_verb = toks
        .iter()
        .any(|t| ["idf.py", "pio", "platformio", "west"].contains(&t.as_str()));
    if needs_verb {
        return toks
            .iter()
            .any(|t| ["flash", "upload", "run"].contains(&t.as_str()));
    }
    true
}

/// Hardware keywords present in text, for aiming a search.
fn hw_terms(text: &str) -> Vec<String> {
    let low = text.to_lowercase();
    let mut found: Vec<String> = Vec::new();
    for w in HW_WORDS {
        if low.contains(w) && !found.iter().any(|f| f.contains(w)) {
            found.push(w.to_string());
        }
    }
    found
}

/// Has a search already happened this session? Reads the journal.
fn searched_this_session(ctx: &Ctx, session: &str) -> bool {
    let path = ctx.paths.sargineer_dir.join("journal.jsonl");
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    for line in text.lines().rev().take(400) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if s(&v, "session") != session {
            continue;
        }
        let verb = s(&v, "verb");
        if ["ask", "preflight", "id"].contains(&verb)
            && v.get("ok").and_then(|b| b.as_bool()) == Some(true)
        {
            return true;
        }
    }
    false
}

/// Was this exact command already guard-blocked this session?
fn already_blocked(ctx: &Ctx, session: &str, cmd: &str) -> bool {
    let path = ctx.paths.sargineer_dir.join("journal.jsonl");
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    for line in text.lines().rev().take(400) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if s(&v, "session") == session
            && v.get("blocked").and_then(|b| b.as_bool()) == Some(true)
            && s(&v, "guard_cmd") == cmd
        {
            return true;
        }
    }
    false
}

fn compact_hits(notes: &[Value], width: usize) -> String {
    let mut out = String::new();
    for (i, n) in notes.iter().take(4).enumerate() {
        out.push_str(&format!(
            "  {}. [{}] {} — {}\n",
            i + 1,
            s(n, "status"),
            crate::render::note_ref(n),
            truncate(s(n, "title"), width.saturating_sub(12))
        ));
        let fix = s(n, "fix");
        if !fix.is_empty() {
            out.push_str(&format!(
                "     fix: {}\n",
                truncate(fix, width.saturating_sub(12))
            ));
        }
    }
    out
}

/// The guard: flash commands must be preceded by a look at what sarg knows.
pub fn guard(ctx: &Ctx, session: &str, cmd: &str) -> GuardDecision {
    let allow = GuardDecision {
        block: false,
        reason: String::new(),
        hits: String::new(),
        query: String::new(),
    };
    if !is_flash(cmd) {
        return allow;
    }
    if searched_this_session(ctx, session) || already_blocked(ctx, session, cmd) {
        return allow;
    }
    let mut terms = hw_terms(cmd);
    if terms.is_empty() {
        // the project's manifest knows which boards are on the bench
        if let Some(p) = crate::manifest::discover() {
            terms = p.manifest.boards.clone();
        }
    }
    let query = if terms.is_empty() {
        // fall back to the tool name so the search is at least on-topic
        tokenize(cmd)
            .into_iter()
            .find(|t| {
                let base = t.rsplit('/').next().unwrap_or(t).to_string();
                FLASH_TOOLS.contains(&base.as_str())
            })
            .unwrap_or_else(|| "flash".into())
    } else {
        terms.join(" ")
    };
    let notes = ctx
        .client
        .get_json(&format!("/notes?q={}&full=1&n=5", urlencode(&query)))
        .ok()
        .and_then(|v| v.get("results").and_then(|r| r.as_array()).cloned())
        .unwrap_or_default();
    let mut notes = notes;
    notes.sort_by_key(|n| status_rank(s(n, "status")));
    let width = crate::render::width();
    let hits = compact_hits(&notes, width);
    let reason = if notes.is_empty() {
        format!(
            "sarg has no lessons for \"{query}\" — you are first here. Flash carefully and record what you learn: sarg lesson new"
        )
    } else {
        format!(
            "Before flashing, sarg has {} lesson(s) for \"{query}\". Read them, then run the command again:",
            notes.len()
        )
    };
    GuardDecision {
        block: true,
        reason,
        hits,
        query,
    }
}

/// One line of context for a prompt, or None. Offline and deterministic:
/// it recognises hardware words and points at `sarg ask`, which is what
/// the case-design and dimension misses in the research needed.
pub fn hint(prompt: &str) -> Option<String> {
    // Only worth a line when the prompt is about building with hardware.
    let intent = [
        "flash",
        "bring up",
        "bring-up",
        "wire",
        "solder",
        "i2c",
        "spi",
        "uart",
        "sensor",
        "camera",
        "case",
        "enclosure",
        "dimension",
        "pinout",
        "footprint",
        "firmware",
        "datasheet",
        "3d print",
        "bracket",
        "mount",
    ];
    let low = prompt.to_lowercase();
    let terms = hw_terms(prompt);
    let intent_hit = intent.iter().any(|w| low.contains(w));
    if terms.is_empty() && !intent_hit {
        return None;
    }
    let q = if terms.is_empty() {
        // pick the first intent word as a weak query
        intent
            .iter()
            .find(|w| low.contains(*w))
            .map(|s| s.to_string())
            .unwrap_or_default()
    } else {
        terms.join(" ")
    };
    if q.is_empty() {
        return None;
    }
    Some(format!(
        "sarg: this touches hardware — someone may have paid for the answer. Search first: `sarg ask {}`",
        shell_quote(&q)
    ))
}

fn shell_quote(s: &str) -> String {
    if s.chars().any(|c| c.is_whitespace()) {
        format!("'{s}'")
    } else {
        s.to_string()
    }
}

/// A stop-time nudge: did this session do hardware work and save nothing?
pub fn nudge(ctx: &Ctx, session: &str) -> Option<String> {
    let path = ctx.paths.sargineer_dir.join("journal.jsonl");
    let text = fs::read_to_string(path).ok()?;
    let mut flashed = false;
    let mut saved = false;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for line in text.lines().rev().take(600) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if s(&v, "session") != session {
            continue;
        }
        let verb = s(&v, "verb");
        seen.insert(verb.to_string());
        if v.get("guard_cmd").is_some() {
            flashed = true;
        }
        if verb.starts_with("lesson.new")
            || verb.starts_with("issue")
            || verb.starts_with("feedback")
        {
            saved = true;
        }
    }
    if flashed && !saved {
        Some("sarg: you did board work this session and saved no lesson — `sarg lesson new` while it is fresh".into())
    } else {
        None
    }
}
