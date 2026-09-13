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
        .env("USERPROFILE", &sb.home)
        .env("SARG_NO_JOURNAL", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    keep_os_env(&mut cmd);
    let mut child = cmd.spawn().unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    // A hold is a decision, not a crash: exit 0 with a deny on stdout.
    assert_eq!(out.status.code(), Some(0), "no hook error");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let h = &v["hookSpecificOutput"];
    assert_eq!(h["hookEventName"], "PreToolUse");
    assert_eq!(h["permissionDecision"], "deny");
    let reason = h["permissionDecisionReason"].as_str().unwrap();
    assert!(reason.contains("Before flashing"), "{reason}");
    assert!(reason.contains("flash-meshcore"), "hits carried: {reason}");
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
        .env("USERPROFILE", &sb.home)
        .env("SARG_NO_JOURNAL", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    keep_os_env(&mut cmd);
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
    #[cfg(windows)]
    assert!(out.contains("Device Manager"), "Windows USB guidance: {out}");
    #[cfg(target_os = "linux")]
    assert!(out.contains("/dev/ttyACM0"), "Linux USB guidance: {out}");
    #[cfg(all(not(target_os = "linux"), not(windows)))]
    assert!(out.contains("sarg id [vid:pid]")
            && !out.contains("/dev/ttyACM0")
            && !out.contains("Device Manager"),
            "non-Linux USB guidance: {out}");
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


#[test]
fn guard_does_not_hold_a_command_that_only_mentions_a_tool() {
    let sb = Sandbox::new();
    // No mock is registered: reaching the server at all would be the bug.
    for cmd in [
        vec!["guard", "grep", "-rn", "mpremote", "src"],
        vec!["guard", "sarg", "ask", "mpremote"],
        vec!["guard", "west", "--help"],
    ] {
        let o = sb.sarg_in(&cmd);
        assert_eq!(code(&o), 0, "{cmd:?} held: {}", stderr(&o));
        assert!(
            stderr(&o).is_empty(),
            "{cmd:?} said something: {}",
            stderr(&o)
        );
    }
}

#[test]
fn guard_allows_when_sarg_knows_nothing_and_says_so_without_holding() {
    let sb = Sandbox::new();
    sb.json_get("/notes", r#"{"results":[],"total":0}"#);
    let env = [("SARG_SESSION", "s-guard-empty")];
    let o = sb.sarg_env(
        &["--token", "t", "guard", "esptool", "write-flash", "0x0", "x.bin"],
        &env,
    );
    assert_eq!(code(&o), 0, "nothing to read is nothing to hold for");
}

#[test]
fn guard_allows_when_sargineer_cannot_be_reached() {
    let sb = Sandbox::new();
    sb.server.mock(|when, then| {
        when.method(GET).path("/notes");
        then.status(500).body("boom");
    });
    let env = [("SARG_SESSION", "s-guard-down")];
    let o = sb.sarg_env(
        &["--token", "t", "guard", "esptool", "write-flash", "0x0", "x.bin"],
        &env,
    );
    assert_eq!(code(&o), 0, "sarg's own trouble must not stop the agent");
    assert!(
        !stderr(&o).contains("you are first here"),
        "must not claim knowledge it does not have: {}",
        stderr(&o)
    );
}

#[test]
fn the_retry_survives_an_edit_to_the_rest_of_the_line() {
    let sb = Sandbox::new();
    sb.json_get("/notes", NOTES_FULL);
    let env = [("SARG_SESSION", "s-guard-key")];
    let o = sb.sarg_env(
        &[
            "--token", "t", "guard", "esptool", "write-flash", "0x0", "run-1.bin",
        ],
        &env,
    );
    assert_eq!(code(&o), 2, "first flash is held: {}", stderr(&o));
    // Same tool, same board, different file: the agent already read them.
    let o = sb.sarg_env(
        &[
            "--token", "t", "guard", "esptool", "write-flash", "0x0", "run-2.bin",
        ],
        &env,
    );
    assert_eq!(
        code(&o),
        0,
        "a reworded retry must not be held again: {}",
        stderr(&o)
    );
}

#[cfg(unix)]
#[test]
fn a_search_piped_into_head_still_counts_as_a_search() {
    let sb = Sandbox::new();
    sb.json_get("/search", r#"{"parts":[],"notes":[]}"#);
    sb.json_get("/notes", NOTES_FULL);
    let env = [("SARG_SESSION", "s-guard-pipe")];
    sb.sarg_piped_into_head(&["--token", "t", "ask", "heltec"], &env);
    let o = sb.sarg_env(
        &["--token", "t", "guard", "esptool", "write-flash", "0x0", "x.bin"],
        &env,
    );
    assert_eq!(
        code(&o),
        0,
        "the search happened; the guard must see it: {}",
        stderr(&o)
    );
}
