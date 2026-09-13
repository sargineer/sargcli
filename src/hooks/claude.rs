//! The Claude Code hook adapter. Reads Claude's hook JSON on stdin and
//! signals back the way Claude expects for each event. This is the only
//! Claude-specific code in the hook path; the decisions come from `core`.
//!
//! Signalling:
//! - PreToolUse: JSON on stdout with exit 0. `permissionDecision: "deny"`
//!   holds the tool and shows Claude the reason; `additionalContext` alone
//!   lets the call through with something worth reading next to it. Exit 2
//!   would also block, but Claude renders it as a hook *error* — a stale
//!   assumption about a board is not a crash, and should not look like one.
//! - UserPromptSubmit: stdout on exit 0 is added to the prompt context.
//! - Stop: we never block; the reminder goes to stderr on exit 0.

use std::io::Read;

use serde_json::{json, Value};

use super::core;
use crate::cli::ClaudeEvent;
use crate::commands::Ctx;
use crate::error::Result;

/// One line of JSON on stdout. A hook's output is read by a parent that may
/// have stopped listening; that is never worth failing a tool call over.
fn emit(v: Value) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

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
            // The guard reasons about the session Claude names; record that
            // one, not whatever the environment happens to carry, or the
            // retry exemption looks at a session that never existed.
            crate::journal::note("session", sess.clone());
            if !d.tool.is_empty() {
                crate::journal::note("guard_tool", d.tool.clone());
                crate::journal::note("guard_key", d.key.clone());
            }
            if d.block {
                let mut reason = d.reason.clone();
                if !d.hits.is_empty() {
                    reason.push('\n');
                    reason.push_str(&d.hits);
                }
                reason.push_str(&format!(
                    "(sarg guard — held once for this board. Run the command again to \
                     proceed, or `sarg ask {}` for the lessons in full.)",
                    d.query
                ));
                emit(json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": reason,
                    }
                }));
                return Ok(0);
            }
            // Nothing to hold for, but sometimes something to say.
            if !d.reason.is_empty() {
                emit(json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "additionalContext": d.reason,
                    }
                }));
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
            crate::journal::note("session", sess.clone());
            if let Some(line) = core::nudge(ctx, &sess) {
                eprintln!("{line}");
            }
            Ok(0)
        }
    }
}
