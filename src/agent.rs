//! Which agent is driving sarg. Detection is explicit and inspectable:
//! `SARG_AGENT` wins, then each agent's own environment markers, then
//! "human" when stdin is a terminal. Every journal line and every hook
//! adapter names the agent it saw, so a wrong guess is visible in
//! `sarg doctor` rather than silent.
//!
//! Adding an agent: add a variant, a marker in `detect`, and — if it has a
//! hook protocol — an adapter module under `hooks/`.

use std::io::IsTerminal;

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Agent {
    /// Claude Code (CLI, desktop, IDE). Markers verified 2026-09-02:
    /// `CLAUDECODE=1`, `CLAUDE_CODE_SESSION_ID`, `AI_AGENT=claude-code_<ver>_agent`.
    ClaudeCode,
    /// OpenAI Codex CLI. Marker unverified; set `SARG_AGENT=codex` until it is.
    Codex,
    /// Cursor agent. Marker unverified; set `SARG_AGENT=cursor` until it is.
    Cursor,
    /// opencode. Marker unverified; set `SARG_AGENT=opencode` until it is.
    Opencode,
    /// A person at a terminal.
    Human,
    /// Named by `SARG_AGENT` or `AI_AGENT` but not one we know.
    Other(String),
    /// No marker and no terminal: a script, a cron job, a pipe.
    Unknown,
}

impl Agent {
    pub fn name(&self) -> &str {
        match self {
            Agent::ClaudeCode => "claude-code",
            Agent::Codex => "codex",
            Agent::Cursor => "cursor",
            Agent::Opencode => "opencode",
            Agent::Human => "human",
            Agent::Other(s) => s.as_str(),
            Agent::Unknown => "unknown",
        }
    }

    pub fn from_name(s: &str) -> Agent {
        match s.trim().to_lowercase().as_str() {
            "claude" | "claude-code" | "claudecode" => Agent::ClaudeCode,
            "codex" => Agent::Codex,
            "cursor" => Agent::Cursor,
            "opencode" => Agent::Opencode,
            "human" => Agent::Human,
            "" | "unknown" => Agent::Unknown,
            other => Agent::Other(other.to_string()),
        }
    }

    /// Is this a coding agent rather than a person or a script?
    #[allow(dead_code)] // the phase 2 hooks branch on this
    pub fn is_agent(&self) -> bool {
        !matches!(self, Agent::Human | Agent::Unknown)
    }

    /// The agent's own session id when its environment carries one.
    pub fn session_id(&self) -> Option<String> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        get("SARG_SESSION").or_else(|| match self {
            Agent::ClaudeCode => get("CLAUDE_CODE_SESSION_ID"),
            _ => None,
        })
    }
}

fn env_set(k: &str) -> bool {
    std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false)
}

pub fn detect() -> Agent {
    if let Ok(v) = std::env::var("SARG_AGENT") {
        if !v.is_empty() {
            return Agent::from_name(&v);
        }
    }
    if env_set("CLAUDECODE") || env_set("CLAUDE_CODE_SESSION_ID") {
        return Agent::ClaudeCode;
    }
    if let Ok(v) = std::env::var("AI_AGENT") {
        // e.g. "claude-code_2-1-259_agent"; keep the product name only.
        let head = v.split('_').next().unwrap_or("").to_string();
        if !head.is_empty() {
            return Agent::from_name(&head);
        }
    }
    if env_set("CODEX_SANDBOX") || env_set("CODEX_CLI") {
        return Agent::Codex;
    }
    if env_set("CURSOR_AGENT") || env_set("CURSOR_TRACE_ID") {
        return Agent::Cursor;
    }
    if env_set("OPENCODE") || env_set("OPENCODE_SESSION") {
        return Agent::Opencode;
    }
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        return Agent::Human;
    }
    Agent::Unknown
}
