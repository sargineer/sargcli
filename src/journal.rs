//! `~/.sargineer/journal.jsonl`: one line per CLI call, so `sarg stats` can
//! later answer "did we ask before flashing" without mining transcripts.
//! Never carries the token. Failures to write are ignored on purpose.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::config::Paths;

static EXTRA: Mutex<Option<Map<String, Value>>> = Mutex::new(None);

/// Attach a field to this call's journal line (query, hit count, …).
pub fn note<V: Into<Value>>(key: &str, value: V) {
    if let Ok(mut g) = EXTRA.lock() {
        g.get_or_insert_with(Map::new)
            .insert(key.to_string(), value.into());
    }
}

pub fn record(paths: Option<&Paths>, verb: &str, code: u8, elapsed: Duration) {
    if std::env::var_os("SARG_NO_JOURNAL").is_some() {
        return;
    }
    let Some(paths) = paths else { return };
    let agent = crate::agent::detect();
    let mut m = Map::new();
    m.insert(
        "ts".into(),
        chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            .into(),
    );
    m.insert("agent".into(), agent.name().into());
    m.insert(
        "session".into(),
        agent.session_id().unwrap_or_else(|| "-".into()).into(),
    );
    m.insert("verb".into(), verb.into());
    m.insert("ok".into(), (code == 0).into());
    m.insert("code".into(), code.into());
    m.insert("ms".into(), (elapsed.as_millis() as u64).into());
    m.insert(
        "cwd".into(),
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    if let Ok(mut g) = EXTRA.lock() {
        if let Some(extra) = g.take() {
            for (k, v) in extra {
                m.insert(k, v);
            }
        }
    }
    let Ok(line) = serde_json::to_string(&Value::Object(m)) else {
        return;
    };
    let _ = std::fs::create_dir_all(&paths.sargineer_dir);
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.sargineer_dir.join("journal.jsonl"))
    {
        let _ = writeln!(f, "{line}");
    }
}
