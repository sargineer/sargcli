//! Guard, hint, the Claude adapter, and hook install — against a mock
//! sargineer and a temp HOME.

mod common;

use std::fs;

use common::*;
use httpmock::prelude::*;

const NOTES_FULL: &str = include_str!("fixtures/notes_full.json");

#[test]
fn guard_allows_non_flash_commands_silently() {
    let sb = Sandbox::new();
    let o = sb.sarg_in(&["guard", "cargo", "build", "--release"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).is_empty() && stderr(&o).is_empty(), "silent: {:?}", (stdout(&o), stderr(&o)));
}

#[test]
fn guard_blocks_a_flash_without_a_prior_search_then_allows_the_retry() {
    let sb = Sandbox::new();
    let notes = sb.server.mock(|when, then| {
        when.method(GET).path("/notes").query_param_exists("q");
        then.status(200).header("content-type", "application/json").body(NOTES_FULL);
    });
    // A session id so guard can track state across calls.
    let env = [("SARG_SESSION", "s-guard-1")];
    let o = sb.sarg_env(&["--token", "t", "guard", "esptool", "--chip", "esp32s3", "write-flash", "0x0", "app.bin"], &env);
    assert_eq!(code(&o), 2, "flash should block: {}", stderr(&o));
    assert!(stderr(&o).contains("Before flashing"), "{}", stderr(&o));
    assert!(stderr(&o).contains("flash-meshcore"), "hits shown: {}", stderr(&o));
    notes.assert_hits(1);

    // Same command again this session → already blocked once, allow.
    let o = sb.sarg_env(&["--token", "t", "guard", "esptool", "--chip", "esp32s3", "write-flash", "0x0", "app.bin"], &env);
    assert_eq!(code(&o), 0, "second time allowed: {}", stderr(&o));
}

#[test]
fn guard_allows_a_flash_after_an_ask_this_session() {
    let sb = Sandbox::new();
    sb.json_get("/search", r#"{"parts":[],"notes":[]}"#);
    sb.json_get("/notes", NOTES_FULL);
    let env = [("SARG_SESSION", "s-guard-2")];
    // ask records a search for this session
    let o = sb.sarg_env(&["--token", "t", "ask", "heltec"], &env);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // now a flash is allowed without a block
    let o = sb.sarg_env(&["--token", "t", "guard", "esptool", "write-flash", "0x0", "x.bin"], &env);
    assert_eq!(code(&o), 0, "allowed after ask: {}", stderr(&o));
}

#[test]
fn hint_fires_on_hardware_intent_and_is_quiet_otherwise() {
    let sb = Sandbox::new();
    let o = sb.sarg(&["hint", "design a case for the esp32-s3-cam"]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).contains("sarg ask"), "{}", stdout(&o));
    assert!(stdout(&o).contains("esp32-s3"), "{}", stdout(&o));

    let o = sb.sarg(&["hint", "write me a limerick about tuesday"]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).trim().is_empty(), "quiet: {}", stdout(&o));
}

#[test]
fn claude_pretooluse_adapter_reads_stdin_and_blocks() {
    let sb = Sandbox::new();
    sb.json_get("/notes", NOTES_FULL);
    let payload = r#"{"session_id":"cc-1","tool_name":"Bash","tool_input":{"command":"esptool write-flash 0x0 heltec.bin for esp32-s3"}}"#;
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_sarg"));
    cmd.args(["--url", &sb.server.base_url(), "--token", "t", "hook", "claude", "pretooluse"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &sb.home)
        .env("SARG_NO_JOURNAL", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2), "block via exit 2");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Before flashing"), "{err}");
}

#[test]
fn claude_pretooluse_allows_non_bash_tools() {
    let sb = Sandbox::new();
    let payload = r#"{"session_id":"cc-2","tool_name":"Read","tool_input":{"file_path":"/x"}}"#;
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_sarg"));
    cmd.args(["--url", &sb.server.base_url(), "hook", "claude", "pretooluse"])
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &sb.home)
        .env("SARG_NO_JOURNAL", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn hook_install_is_idempotent_and_backs_up() {
    let sb = Sandbox::new();
    let claude = sb.home.join(".claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("settings.json"), r#"{"env":{"FOO":"bar"}}"#).unwrap();

    let o = sb.sarg(&["hook", "install"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let settings = fs::read_to_string(claude.join("settings.json")).unwrap();
    assert!(settings.contains("sarg hook claude pretooluse"), "{settings}");
    assert!(settings.contains("\"matcher\": \"Bash\""), "{settings}");
    assert!(settings.contains("\"FOO\""), "kept existing keys: {settings}");
    assert!(claude.join("settings.json.sarg-bak").exists(), "backup made");

    // Second run adds nothing.
    let o = sb.sarg(&["hook", "install"]);
    assert!(stdout(&o).contains("already installed"), "{}", stdout(&o));

    // Status sees them; uninstall removes them.
    let o = sb.sarg(&["hook", "status"]);
    assert!(stdout(&o).contains("PreToolUse"), "{}", stdout(&o));
    let o = sb.sarg(&["hook", "uninstall"]);
    assert_eq!(code(&o), 0);
    let settings = fs::read_to_string(claude.join("settings.json")).unwrap();
    assert!(!settings.contains("sarg hook claude"), "{settings}");
    assert!(settings.contains("\"FOO\""));
}

#[test]
fn doctor_flags_stale_pointers_and_missing_pieces() {
    let sb = Sandbox::new();
    sb.json_get("/api", include_str!("fixtures/api.json"));
    let claude = sb.home.join(".claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("CLAUDE.md"), "the sargineer server runs on localhost:8093\n").unwrap();
    let o = sb.sarg(&["doctor"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("localhost:8093"), "{out}");
    assert!(out.contains("no token") || out.contains("hooks not installed"), "{out}");
}

#[test]
fn skill_emits_delegating_markdown_with_config_values() {
    let sb = Sandbox::new();
    sb.sarg(&["config", "set", "host_tags", "linux,x86_64"]);
    sb.sarg(&["config", "set", "deny_terms", "acme"]);
    let o = sb.sarg(&["skill"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("name: sarg"), "{out}");
    assert!(out.contains("sarg ask"), "{out}");
    assert!(out.contains("sarg lesson new"), "{out}");
    assert!(out.contains("linux, x86_64"), "host tags carried: {out}");
    assert!(out.contains("acme"), "deny carried: {out}");
    assert!(out.contains("Publishing is always the user's"), "{out}");
}

#[test]
fn stats_summarizes_the_journal() {
    let sb = Sandbox::new();
    let journal = sb.home.join(".sargineer/journal.jsonl");
    fs::create_dir_all(journal.parent().unwrap()).unwrap();
    fs::write(
        &journal,
        concat!(
            r#"{"ts":"t","agent":"claude-code","session":"s1","verb":"ask","ok":true}"#, "\n",
            r#"{"ts":"t","agent":"claude-code","session":"s1","verb":"guard","ok":true,"guard_cmd":"esptool","blocked":true}"#, "\n",
            r#"{"ts":"t","agent":"human","session":"s2","verb":"lesson.new","ok":true}"#, "\n",
        ),
    )
    .unwrap();
    let o = sb.sarg(&["stats"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("3 calls"), "{out}");
    assert!(out.contains("held for a search"), "{out}");

    let o = sb.sarg(&["--json", "stats"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["guard_blocks"], 1);
    assert_eq!(v["notes_written"], 1);
    assert_eq!(v["sessions"], 2);
}
