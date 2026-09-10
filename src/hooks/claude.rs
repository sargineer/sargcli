//! The Claude Code hook adapter. Reads Claude's hook JSON on stdin and
//! signals back the way Claude expects for each event. This is the only
//! Claude-specific code in the hook path; the decisions come from `core`.
//!
//! Signalling (kept to the robust, version-stable forms):
//! - PreToolUse: exit 2 with stderr blocks the tool and feeds stderr back
//!   to Claude; exit 0 allows.
//! - UserPromptSubmit: stdout on exit 0 is added to the prompt context.
//! - Stop: we never block; the reminder goes to stderr on exit 0.

use std::io::Read;

use serde_json::Value;

use super::core;
use crate::cli::ClaudeEvent;
use crate::commands::Ctx;
use crate::error::Result;

fn read_stdin() -> Value {
    let mut buf = String::new();
    let _ = std::io::stdin().read_to_string(&mut buf);
    serde_json::from_str(&buf).unwrap_or(Value::Null)
}

fn field<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            return s;
        }
    }
    ""
}

/// Session id from the hook payload, then the environment, then "-".
fn session(v: &Value) -> String {
    let s = field(v, &["session_id", "sessionId"]);
    if !s.is_empty() {
        return s.to_string();
    }
    crate::agent::Agent::ClaudeCode
        .session_id()
        .unwrap_or_else(|| "-".into())
}

/// Returns the process exit code the adapter should use.
pub fn run(ctx: &mut Ctx, event: ClaudeEvent) -> Result<i32> {
    let payload = read_stdin();
    let sess = session(&payload);
    match event {
        ClaudeEvent::Pretooluse => {
            // Only Bash carries a shell command; anything else is allowed.
            let tool = field(&payload, &["tool_name", "toolName"]);
            let cmd = payload
                .get("tool_input")
                .or_else(|| payload.get("toolInput"))
                .and_then(|ti| ti.get("command"))
                .and_then(|c| c.as_str())
                .unwrap_or("");
            if tool != "Bash" || cmd.is_empty() {
                return Ok(0);
            }
            let d = core::guard(ctx, &sess, cmd);
            crate::journal::note("blocked", d.block);
            crate::journal::note("guard_cmd", cmd.to_string());
            if d.block {
                eprintln!("{}", d.reason);
                if !d.hits.is_empty() {
                    eprint!("{}", d.hits);
                }
                eprintln!("(sarg guard — run `sarg ask {}` or retry to proceed)", d.query);
                return Ok(2);
            }
            Ok(0)
        }
        ClaudeEvent::Userpromptsubmit => {
            let prompt = field(&payload, &["prompt", "user_prompt", "userPrompt"]);
            if let Some(line) = core::hint(prompt) {
                println!("{line}");
            }
            Ok(0)
        }
        ClaudeEvent::Stop => {
            if let Some(line) = core::nudge(ctx, &sess) {
                eprintln!("{line}");
            }
            Ok(0)
        }
    }
}
