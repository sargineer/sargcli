//! Text rendering shared by the read-side verbs. Compact by default: one
//! hit is two or three lines, a full lesson is a screen. Colour only on a
//! terminal; agents read through a pipe and get plain text.

use std::io::IsTerminal;
use std::sync::OnceLock;

use serde_json::Value;

fn color_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
    })
}

fn wrap_ansi(code: &str, s: &str) -> String {
    if color_on() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}
pub fn bold(s: &str) -> String {
    wrap_ansi("1", s)
}
pub fn dim(s: &str) -> String {
    wrap_ansi("2", s)
}
pub fn warn(s: &str) -> String {
    wrap_ansi("33", s)
}
pub fn good(s: &str) -> String {
    wrap_ansi("32", s)
}

/// Field as &str, "" when missing or not a string.
pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}
pub fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}
pub fn list(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .map(|e| match e {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn one_line(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn truncate(t: &str, n: usize) -> String {
    let t = one_line(t);
    if t.chars().count() <= n {
        return t;
    }
    let cut: String = t.chars().take(n.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Word-wrap to `width` with every line indented by `indent` spaces.
pub fn wrap(t: &str, width: usize, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    let mut line = String::new();
    for w in one_line(t).split(' ') {
        if !line.is_empty() && line.chars().count() + 1 + w.chars().count() > width {
            out.push_str(&pad);
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    if !line.is_empty() {
        out.push_str(&pad);
        out.push_str(&line);
    }
    out
}

pub fn human_bytes(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{:.1} MB", n as f64 / 1048576.0),
    }
}

/// working > provisional > unverified > superseded > retracted
pub fn status_rank(st: &str) -> u8 {
    match st {
        "working" => 0,
        "provisional" => 1,
        "unverified" => 2,
        "superseded" => 3,
        "retracted" => 4,
        _ => 5,
    }
}

pub fn status_badge(st: &str) -> String {
    let b = format!("[{st}]");
    match st {
        "working" => good(&b),
        "provisional" | "unverified" => warn(&b),
        "" => dim("[?]"),
        _ => dim(&b),
    }
}

pub fn note_ref(v: &Value) -> String {
    format!("{}/{}", s(v, "handle"), s(v, "id"))
}

/// `[working] sarg/flash-… · hw heltec-v4 esp32-s3 · sw esptool@5.3.1 · 2026-08-20 · private`
pub fn note_header(v: &Value) -> String {
    let mut parts = vec![format!(
        "{} {}",
        status_badge(s(v, "status")),
        bold(&note_ref(v))
    )];
    if v.get("title_only").and_then(|t| t.as_bool()) == Some(true) {
        parts.push(dim("title only — sarg show for the rest"));
    }
    if !s(v, "kind").is_empty() && s(v, "kind") != "lesson" {
        parts.push(dim(&format!("kind {}", s(v, "kind"))));
    }
    let hw = list(v, "hw");
    if !hw.is_empty() {
        parts.push(dim(&format!("hw {}", hw.join(" "))));
    }
    let sw = list(v, "sw");
    if !sw.is_empty() {
        parts.push(dim(&format!("sw {}", sw.join(" "))));
    }
    let date = s(v, "date");
    let date = if date.is_empty() {
        s(v, "updated").chars().take(10).collect::<String>()
    } else {
        date.to_string()
    };
    if !date.is_empty() {
        parts.push(dim(&date));
    }
    if s(v, "visibility") == "private" {
        parts.push(warn("private"));
    }
    parts.join(" · ")
}

/// Two to three lines: header, title, and the fix (or summary) if present.
pub fn note_compact(idx: usize, v: &Value, width: usize) -> String {
    let mut out = format!("{idx:>2}. {}\n", note_header(v));
    let title = truncate(s(v, "title"), width.saturating_sub(4) * 2);
    out.push_str(&wrap(&title, width.saturating_sub(4), 4));
    out.push('\n');
    let fix = s(v, "fix");
    if !fix.is_empty() {
        out.push_str(&format!(
            "    {} {}\n",
            dim("fix:"),
            truncate(fix, width.saturating_sub(9))
        ));
    } else if !s(v, "summary").is_empty() {
        out.push_str(&format!(
            "    {}\n",
            dim(&truncate(s(v, "summary"), width.saturating_sub(4)))
        ));
    } else if !s(v, "symptom").is_empty() {
        out.push_str(&format!(
            "    {} {}\n",
            dim("symptom:"),
            truncate(s(v, "symptom"), width.saturating_sub(13))
        ));
    }
    if !list(v, "unverified").is_empty() {
        out.push_str(&format!("    {}\n", warn("has unverified figures — sarg show for the fence")));
    }
    out
}

/// Everything, in reading order. `unverified` is fenced and labelled so it
/// is never mistaken for the fix.
pub fn note_full(v: &Value, width: usize) -> String {
    let w = width.max(40);
    let mut out = String::new();
    out.push_str(&note_header(v));
    out.push('\n');
    out.push_str(&wrap(&bold(s(v, "title")), w, 0));
    out.push_str("\n\n");

    let field = |out: &mut String, label: &str, text: &str| {
        if text.trim().is_empty() {
            return;
        }
        out.push_str(&format!("{}\n", dim(label)));
        out.push_str(&wrap(text, w.saturating_sub(2), 2));
        out.push_str("\n\n");
    };
    field(&mut out, "setup", s(v, "setup"));
    field(&mut out, "intent", s(v, "intent"));
    field(&mut out, "symptom", s(v, "symptom"));
    field(&mut out, "cause", s(v, "cause"));
    field(&mut out, "fix", s(v, "fix"));

    let steps = list(v, "steps");
    if !steps.is_empty() {
        out.push_str(&format!("{}\n", dim("steps")));
        for (i, st) in steps.iter().enumerate() {
            let head = format!("  {:>2}. ", i + 1);
            let body = wrap(st, w.saturating_sub(6), 6);
            out.push_str(&head);
            out.push_str(body.trim_start());
            out.push('\n');
        }
        out.push('\n');
    }
    field(&mut out, "check", s(v, "check"));

    let unv = list(v, "unverified");
    if !unv.is_empty() {
        out.push_str(&warn("unverified — nobody confirmed these; never present them as the fix"));
        out.push('\n');
        for u in &unv {
            out.push_str(&warn("  ? "));
            out.push_str(wrap(u, w.saturating_sub(4), 4).trim_start());
            out.push('\n');
        }
        out.push('\n');
    }
    field(&mut out, "body", s(v, "body"));
    field(&mut out, "cost", s(v, "cost"));
    field(&mut out, "about", s(v, "about"));

    let mut meta: Vec<String> = Vec::new();
    for k in ["hw", "sw", "host", "refs"] {
        let l = list(v, k);
        if !l.is_empty() {
            meta.push(format!("{k}: {}", l.join(", ")));
        }
    }
    for k in ["project", "supersedes", "superseded_by", "confidence"] {
        if !s(v, k).is_empty() {
            meta.push(format!("{k}: {}", s(v, k)));
        }
    }
    if !meta.is_empty() {
        out.push_str(&dim(&meta.join("  ·  ")));
        out.push('\n');
    }
    out
}

/// Terminal width for wrapping: $COLUMNS, else 100.
pub fn width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .filter(|w: &usize| *w >= 40)
        .unwrap_or(100)
}

/// One line for a part hit from /search or /parts.
pub fn part_line(p: &Value) -> String {
    // Tag pages list parts as bare ids; search and vendor pages as objects.
    if let Value::String(id) = p {
        return bold(id);
    }
    let id = if s(p, "product").is_empty() {
        s(p, "id")
    } else {
        s(p, "product")
    };
    let mut bits = vec![bold(id)];
    let name = s(p, "name");
    if !name.is_empty() {
        bits.push(truncate(name, 60));
    }
    let models = u(p, "models");
    if models > 0 {
        bits.push(dim(&format!("{models} model{}", if models == 1 { "" } else { "s" })));
    }
    if let Some(l) = p.get("lessons").and_then(|l| l.get("count")).and_then(|c| c.as_u64()) {
        bits.push(dim(&format!("{l} lessons")));
    }
    if p.get("yours").and_then(|y| y.as_bool()) == Some(true) {
        bits.push(good("owned"));
    }
    bits.join(" · ")
}
