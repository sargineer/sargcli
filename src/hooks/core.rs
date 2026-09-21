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
    /// One-line reason. Set when blocking, and also when allowing with
    /// something worth saying — the adapters decide how to carry it.
    pub reason: String,
    /// Compact hits to show, already rendered.
    pub hits: String,
    pub query: String,
    /// The flash tool this command runs, when it runs one. Journalled, so
    /// the stop-time nudge knows board work happened whether or not the
    /// agent was held for it.
    pub tool: String,
    /// What the hold was about: `<tool>:<boards>`. Deliberately free of
    /// anything taken from the command line, so editing a filename or a
    /// flag does not earn a second stop.
    pub key: String,
}

impl GuardDecision {
    fn allow() -> GuardDecision {
        GuardDecision {
            block: false,
            reason: String::new(),
            hits: String::new(),
            query: String::new(),
            tool: String::new(),
            key: String::new(),
        }
    }
}

/// Commands that only wrap another command; the real one follows.
const WRAPPERS: [&str; 12] = [
    "sudo", "doas", "env", "command", "exec", "nohup", "nice", "ionice", "stdbuf", "time",
    "timeout", "xargs",
];

/// Wrapper options that swallow the next word, so it is not the command.
const WRAPPER_VALUE_FLAGS: [&str; 14] = [
    "-u", "-g", "-C", "-p", "-S", "-s", "--signal", "-k", "-n", "-c", "-i", "-o", "-e", "-I",
];

/// Shells: what they are told to run is code, not an argument.
const SHELLS: [&str; 8] = ["bash", "sh", "zsh", "dash", "ksh", "eval", "ssh", "su"];
/// Interpreters: only `-m <module>` runs a tool.
const INTERPRETERS: [&str; 3] = ["python", "python3", "py"];
/// Package runners: `uv run <tool>`, `uvx <tool>`, `npx <tool>`, …
const PKG_RUNNERS: [&str; 8] = ["uv", "uvx", "pipx", "poetry", "conda", "npx", "pdm", "rye"];

/// `NAME=value`, the shell's own prefix assignment.
fn is_assignment(w: &str) -> bool {
    let Some((name, _)) = w.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `20`, `5s` — a wrapper's own numeric argument, as in `timeout 20 <tool>`.
fn is_duration(w: &str) -> bool {
    let core = w.strip_suffix(['s', 'm', 'h', 'd']).unwrap_or(w);
    !core.is_empty() && core.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn is_meta(c: char) -> bool {
    ";|&\n(){}`<>".contains(c)
}

/// Shell text with the parts that are prose rather than code removed, and
/// everything else kept as written.
///
/// Quoting is *unwrapped*, not deleted: `mp""remote` has to stay one word,
/// and `bash -c "esptool …"` has to stay visible. Only the metacharacters
/// inside quotes are neutralised, so a quoted `;` cannot invent a command.
/// Comments and heredoc bodies are dropped — that is where the prose lives,
/// and a heredoc full of house plans reading as a flash is what started
/// this — unless the line opening the heredoc feeds a shell.
fn strip_noncode(cmd: &str) -> String {
    let c: Vec<char> = cmd.chars().collect();
    let n = c.len();
    let mut out = String::with_capacity(cmd.len());
    let mut i = 0usize;
    // Heredoc delimiters opened on this line, consumed at the next newline.
    let mut pending: Vec<(String, bool)> = Vec::new();
    // Where the current line starts in `out`, to see what opened a heredoc.
    let mut line_start = 0usize;
    while i < n {
        match c[i] {
            // `$'…'` is a single-quoted string that honours backslashes.
            '$' if c.get(i + 1) == Some(&'\'') => {
                i += 2;
                while i < n && c[i] != '\'' {
                    if c[i] == '\\' {
                        i += 1;
                    }
                    if i < n {
                        out.push(if is_meta(c[i]) { ' ' } else { c[i] });
                        i += 1;
                    }
                }
                i = (i + 1).min(n);
            }
            '\'' => {
                i += 1;
                while i < n && c[i] != '\'' {
                    out.push(if is_meta(c[i]) { ' ' } else { c[i] });
                    i += 1;
                }
                i = (i + 1).min(n);
            }
            '"' => {
                i += 1;
                while i < n && c[i] != '"' {
                    if c[i] == '\\' && i + 1 < n {
                        i += 1;
                    }
                    out.push(if is_meta(c[i]) { ' ' } else { c[i] });
                    i += 1;
                }
                i = (i + 1).min(n);
            }
            '\\' => {
                // A backslash quotes the next character; a newline it quotes
                // is a line continuation and disappears.
                match c.get(i + 1) {
                    Some('\n') | None => out.push(' '),
                    Some(&ch) => out.push(if is_meta(ch) { ' ' } else { ch }),
                }
                i = (i + 2).min(n);
            }
            // A comment opens a word; `${#x}` and `$#` are not comments.
            '#' if out
                .chars()
                .next_back()
                .is_none_or(|p| p.is_whitespace() || ";|&(".contains(p)) =>
            {
                while i < n && c[i] != '\n' {
                    i += 1;
                }
            }
            // `<<WORD` / `<<-WORD` open a heredoc. `<<<` is a here-string and
            // `$(( a << b ))` is a shift.
            '<' if c.get(i + 1) == Some(&'<')
                && c.get(i + 2) != Some(&'<')
                && !out.ends_with('(') =>
            {
                i += 2;
                if c.get(i) == Some(&'-') {
                    i += 1;
                }
                while i < n && (c[i] == ' ' || c[i] == '\t') {
                    i += 1;
                }
                let mut delim = String::new();
                while i < n && !c[i].is_whitespace() && !is_meta(c[i]) {
                    if !matches!(c[i], '\'' | '"' | '\\') {
                        delim.push(c[i]);
                    }
                    i += 1;
                }
                // A heredoc delimiter is a word. `$(( 1 << 3 ))` would
                // otherwise open one called "3" and swallow the rest.
                let is_word = delim
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
                if is_word {
                    // `ssh pi bash <<EOF` really is feeding a shell.
                    let line = &out[line_start..];
                    let feeds_shell = line
                        .split(|ch: char| ch.is_whitespace() || is_meta(ch))
                        .any(|w| SHELLS.contains(&w.rsplit('/').next().unwrap_or(w)));
                    pending.push((delim, feeds_shell));
                }
                out.push(' ');
            }
            '\n' => {
                out.push('\n');
                i += 1;
                while !pending.is_empty() {
                    let (delim, keep) = pending.remove(0);
                    loop {
                        let start = i;
                        while i < n && c[i] != '\n' {
                            i += 1;
                        }
                        let line: String = c[start..i].iter().collect();
                        let at_end = i >= n;
                        if i < n {
                            i += 1;
                        }
                        if line.trim() == delim || at_end {
                            break;
                        }
                        if keep {
                            out.push_str(&line);
                            out.push('\n');
                        }
                    }
                }
                line_start = out.len();
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// Every command position in shell code: the command word (basename,
/// lowercased) and the rest of that segment's words.
fn command_segments(code: &str) -> Vec<(String, Vec<String>)> {
    let mut segs: Vec<Vec<String>> = vec![Vec::new()];
    let mut word = String::new();
    for ch in code.chars() {
        if is_meta(ch) {
            if !word.is_empty() {
                segs.last_mut().unwrap().push(std::mem::take(&mut word));
            }
            if !segs.last().unwrap().is_empty() {
                segs.push(Vec::new());
            }
        } else if ch.is_whitespace() {
            if !word.is_empty() {
                segs.last_mut().unwrap().push(std::mem::take(&mut word));
            }
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        segs.last_mut().unwrap().push(word);
    }

    let mut out = Vec::new();
    for seg in segs {
        if let Some(cmd) = peel(seg) {
            out.push(cmd);
        }
    }
    out
}

/// Drop prefix assignments and wrapper commands until the real command word
/// is in front: `A=1 sudo -u pi timeout 20 /usr/bin/tool erase` → `tool`.
fn peel(seg: Vec<String>) -> Option<(String, Vec<String>)> {
    let mut it = seg.into_iter().peekable();
    while let Some(w) = it.peek().cloned() {
        if is_assignment(&w) {
            it.next();
            continue;
        }
        let base = w.rsplit('/').next().unwrap_or(&w).to_lowercase();
        if WRAPPERS.contains(&base.as_str()) {
            it.next();
            // `command -v tool` asks where a tool is; it does not run it.
            if base == "command" && it.peek().is_some_and(|p| p == "-v" || p == "-V") {
                return None;
            }
            while let Some(p) = it.peek().cloned() {
                if is_assignment(&p) || is_duration(&p) {
                    it.next();
                } else if p.starts_with('-') {
                    it.next();
                    if WRAPPER_VALUE_FLAGS.contains(&p.as_str()) {
                        it.next();
                    }
                } else {
                    break;
                }
            }
            continue;
        }
        break;
    }
    let w = it.next()?;
    let base = w.rsplit('/').next().unwrap_or(&w).to_lowercase();
    Some((base, it.map(|w| w.to_lowercase()).collect()))
}

/// A runner's arguments, reduced to the command it will actually run.
/// `bash -c esptool …` → `esptool …`; `python -m mpremote …` → `mpremote …`;
/// `uv run esptool …` → `esptool …`. `None` when it runs nothing of ours.
fn inner_command(name: &str, rest: &[String]) -> Option<Vec<String>> {
    let mut it = rest.iter().peekable();
    if SHELLS.contains(&name) {
        while it.peek().is_some_and(|w| w.starts_with('-')) {
            it.next();
        }
        // `ssh host <command>` / `su user -c <command>`: the first bare word
        // names where to run, not what to run.
        if name == "ssh" || name == "su" {
            it.next();
            while it.peek().is_some_and(|w| w.starts_with('-')) {
                it.next();
            }
        }
    } else if INTERPRETERS.contains(&name) {
        loop {
            let w = it.peek()?.as_str();
            if w == "-m" {
                it.next();
                break;
            }
            if !w.starts_with('-') {
                return None; // running a script, not a module
            }
            it.next();
        }
    } else if PKG_RUNNERS.contains(&name) {
        while it.peek().is_some_and(|w| w.starts_with('-')) {
            it.next();
        }
        if it
            .peek()
            .is_some_and(|w| ["run", "exec", "tool"].contains(&w.as_str()))
        {
            it.next();
        }
        while it.peek().is_some_and(|w| w.starts_with('-')) {
            it.next();
        }
    } else {
        return None;
    }
    let v: Vec<String> = it.cloned().collect();
    (!v.is_empty()).then_some(v)
}

/// Is this command word a flash tool, given the rest of its line?
fn flashes(name: &str, rest: &[String]) -> bool {
    if !FLASH_TOOLS.contains(&name) {
        return false;
    }
    // idf.py / pio / west are only flashing when they say so.
    if ["idf.py", "pio", "platformio", "west"].contains(&name) {
        return rest
            .iter()
            .any(|t| ["flash", "upload", "run"].contains(&t.as_str()));
    }
    true
}

/// The flash tool this command actually invokes, if any. A tool named in a
/// comment, a heredoc, or an argument to something else is not an invocation
/// — but a tool handed to a shell, an interpreter or a runner is.
pub fn flash_tool(cmd: &str) -> Option<String> {
    fn walk(name: &str, rest: &[String], depth: u8) -> Option<String> {
        // sarg is never held by its own guard — least of all `sarg ask`.
        if name == "sarg" {
            return None;
        }
        if flashes(name, rest) {
            return Some(name.to_string());
        }
        if depth == 0 {
            return None;
        }
        let inner = inner_command(name, rest)?;
        // What a runner runs is a command line of its own.
        for (n, r) in command_segments(&inner.join(" ")) {
            if let Some(found) = walk(&n, &r, depth - 1) {
                return Some(found);
            }
        }
        None
    }
    for (name, rest) in command_segments(&strip_noncode(cmd)) {
        if let Some(found) = walk(&name, &rest, 3) {
            return Some(found);
        }
    }
    None
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

/// How far back in the journal to look. It is shared by every session on
/// the machine, so a busy afternoon can put a lot between us and our own
/// lines; 400 was thin enough to lose a search that really happened.
const JOURNAL_SCAN: usize = 4000;

/// Journal lines belonging to one session, newest first.
fn session_lines(ctx: &Ctx, session: &str) -> Vec<Value> {
    let path = ctx.paths.sargineer_dir.join("journal.jsonl");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .take(JOURNAL_SCAN)
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| s(v, "session") == session)
        .collect()
}

/// Has a search already happened this session?
fn searched_this_session(ctx: &Ctx, session: &str) -> bool {
    session_lines(ctx, session).iter().any(|v| {
        ["ask", "preflight", "id"].contains(&s(v, "verb"))
            && v.get("ok").and_then(|b| b.as_bool()) == Some(true)
    })
}

/// Were we already held for this same tool and board this session?
fn already_blocked(ctx: &Ctx, session: &str, key: &str) -> bool {
    session_lines(ctx, session).iter().any(|v| {
        v.get("blocked").and_then(|b| b.as_bool()) == Some(true) && s(v, "guard_key") == key
    })
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

/// The guard: a flash should be preceded by a look at what sarg knows.
///
/// It holds only when there is something to read. No lessons, or sarg
/// itself in trouble, means the agent is let through — a guard that stops
/// work while saying nothing teaches agents to route around it.
pub fn guard(ctx: &Ctx, session: &str, cmd: &str) -> GuardDecision {
    let Some(tool) = flash_tool(cmd) else {
        return GuardDecision::allow();
    };
    let boards = crate::manifest::discover()
        .map(|p| p.manifest.boards.clone())
        .unwrap_or_default();
    let mut terms = hw_terms(cmd);
    if terms.is_empty() {
        // the project's manifest knows which boards are on the bench
        terms = boards.clone();
    }
    // Fall back to the tool name so the search is at least on-topic.
    let query = if terms.is_empty() {
        tool.clone()
    } else {
        terms.join(" ")
    };
    // The key names the bench, never the command line: renaming a firmware
    // file must not look like a different question.
    let key = format!("{tool}:{}", boards.join(","));
    let held = GuardDecision {
        tool: tool.clone(),
        key: key.clone(),
        ..GuardDecision::allow()
    };
    if searched_this_session(ctx, session) || already_blocked(ctx, session, &key) {
        return held;
    }

    let fetched = ctx
        .client
        .get_json(&format!("/notes?q={}&full=1&n=5", urlencode(&query)));
    let mut notes: Vec<Value> = match fetched {
        Ok(v) => v
            .get("results")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default(),
        // sarg being unreachable or unhappy is sarg's problem, not the
        // agent's. Allow — but leave a trace, or a revoked token silently
        // retires the guard for good.
        Err(e) => {
            crate::journal::note("guard_error", e.to_string());
            return GuardDecision {
                reason: format!("sarg guard skipped — {e}. `sarg doctor` says why."),
                ..held
            };
        }
    };
    if notes.is_empty() {
        return GuardDecision {
            reason: format!(
                "sarg has no lessons for \"{query}\" — you are first here. \
                 Flash carefully and record what you learn: sarg lesson new"
            ),
            query,
            ..held
        };
    }

    notes.sort_by_key(|n| status_rank(s(n, "status")));
    let width = crate::render::width();
    let hits = compact_hits(&notes, width);
    let reason = format!(
        "Before flashing, sarg has {} lesson(s) for \"{query}\". Read them, then run the command again:",
        notes.len()
    );
    GuardDecision {
        block: true,
        reason,
        hits,
        query,
        ..held
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
        if v.get("guard_tool").is_some() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn is_flash(cmd: &str) -> bool {
        flash_tool(cmd).is_some()
    }

    /// Every one of these was a real block in the journal, and none of them
    /// writes to a board.
    #[test]
    fn a_tool_named_in_passing_is_not_a_flash() {
        for cmd in [
            "grep -rn mpremote ~/code/dustycam",
            "git commit -m \"fix mpremote retry\"",
            "cat tools/mp # wraps mpremote",
            "echo \"the word mpremote and the word flash\"",
            "rg --files-with-matches esptool",
            // the remedy the guard itself prescribes
            "sarg ask mpremote",
            "sarg ask openmv 2>&1 | head -1",
            // recording the lesson afterwards
            "sarg lesson new --title \"mpremote hangs on the N6\" --fix \"reset first\"",
            // west and run, as English, in a heredoc full of prose
            "cat >> options.py <<'EOF'\n# the deck runs along the west wall\ndef option_d():\n    pass\nEOF",
            // pio, as the RP2040 peripheral, in a schematic
            "cat > index.circuit.tsx <<'EOF'\n// PIO drives the encoder; run at 125MHz\nEOF",
            // named but not invoked
            "west --help",
            "pio device list",
            "which esptool avrdude",
        ] {
            assert!(!is_flash(cmd), "should not read as a flash: {cmd}");
        }
    }

    #[test]
    fn a_real_invocation_is_still_a_flash() {
        for cmd in [
            "esptool --chip esp32s3 write-flash 0x0 app.bin",
            "timeout 20 mpremote connect $D reset",
            "sudo /usr/bin/avrdude -p m328p -c usbasp",
            "PORT=/dev/ttyACM0 mpremote cp main.py :main.py",
            "D=$(ls /dev/serial/by-id/usb-x-if00); mpremote connect $D reset",
            "cd ~/proj && pio run -t upload",
            "west flash --runner jlink",
            "idf.py -p /dev/ttyUSB0 flash monitor",
            "cat firmware.bin | esptool write-flash 0x0 -",
        ] {
            assert!(is_flash(cmd), "should read as a flash: {cmd}");
        }
    }

    /// Handing a tool to a shell, an interpreter or a runner is running it.
    /// The first pass at this parser missed every one of these.
    #[test]
    fn a_tool_handed_to_a_runner_is_still_a_flash() {
        for cmd in [
            "bash -c \"esptool write-flash 0x0 app.bin\"",
            "sh -c 'mpremote connect /dev/ttyACM0 reset'",
            "eval \"esptool erase-flash\"",
            "ssh pi 'mpremote reset'",
            "python -m esptool write-flash 0x0 app.bin",
            "python3 -m mpremote connect /dev/ttyACM0 reset",
            "uv run esptool erase-flash",
            "uvx esptool erase-flash",
            "npx --yes esptool erase-flash",
            "sudo -u pi mpremote reset",
            "ssh pi bash <<'EOF'\nesptool write-flash 0x0 a.bin\nEOF\n",
        ] {
            assert!(is_flash(cmd), "runner hid a flash: {cmd}");
        }
    }

    /// Quoting is unwrapped, not deleted — this is the evasion the journal
    /// caught an agent using to get past the old guard.
    #[test]
    fn quoting_cannot_split_a_tool_name() {
        for cmd in [
            "mp\"\"remote reset",
            "'mpremote' reset",
            "\"esptool\" erase-flash",
            "mp'r'emote reset",
        ] {
            assert!(is_flash(cmd), "quote split hid a flash: {cmd}");
        }
    }

    /// A runner that runs something else must stay quiet.
    #[test]
    fn a_runner_running_something_else_is_not_a_flash() {
        for cmd in [
            "uv run pytest -k esptool",
            "python -m pytest tests/test_esptool.py",
            "python setup.py build",
            "command -v esptool",
            "ssh pi 'ls /dev'",
        ] {
            assert!(!is_flash(cmd), "should not read as a flash: {cmd}");
        }
    }

    #[test]
    fn the_tool_is_named_so_the_hold_can_be_keyed_on_it() {
        assert_eq!(
            flash_tool("timeout 20 mpremote reset").as_deref(),
            Some("mpremote")
        );
        assert_eq!(
            flash_tool("sudo /usr/bin/esptool erase-flash").as_deref(),
            Some("esptool")
        );
        assert_eq!(flash_tool("ls -la"), None);
    }

    #[test]
    fn heredoc_bodies_and_quotes_are_not_shell_code() {
        let stripped = strip_noncode("cat <<'EOF'\nmpremote everywhere\nEOF\nmpremote reset\n");
        assert!(!stripped.contains("everywhere"), "body kept: {stripped}");
        assert!(stripped.contains("mpremote reset"), "code lost: {stripped}");

        // `$(…)` is code and must survive — it is where a real flash can hide.
        let stripped = strip_noncode("X=$(mpremote reset)");
        assert!(stripped.contains("mpremote"), "subshell lost: {stripped}");
        assert!(is_flash("X=$(mpremote reset)"));
    }

    #[test]
    fn wrappers_and_assignments_are_peeled() {
        let segs = command_segments("A=1 sudo -u pi timeout 20 /usr/bin/esptool erase");
        assert_eq!(segs[0].0, "esptool");
        assert_eq!(segs[0].1, vec!["erase"]);
    }

    /// Shells opened a heredoc, so its body is code; `cat` did not, so the
    /// house plans in it stay prose.
    #[test]
    fn a_heredoc_is_code_only_when_it_feeds_a_shell() {
        assert!(!is_flash(
            "cat >> options.py <<'EOF'\nthe deck runs along the west wall\nEOF\n"
        ));
        assert!(is_flash("bash <<'EOF'\nesptool erase-flash\nEOF\n"));
    }
}
