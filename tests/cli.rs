//! End-to-end tests of the plumbing verbs against a mock sargineer. No real
//! network, no real config: HOME points at a temp directory per test.

mod common;

use std::fs;
use std::path::Path;

use common::*;
use httpmock::prelude::*;

const API_FIXTURE: &str = include_str!("fixtures/api.json");

#[test]
fn auth_status_signed_in_text_and_json() {
    let sb = Sandbox::new();
    let m = sb.server.mock(|when, then| {
        when.method(GET)
            .path("/me")
            .header("authorization", "Bearer tok-123");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"handle":"sargbench2","role":"member","created":"2026-08-20T14:52:01+00:00"}"#);
    });
    let o = sb.sarg_in(&["auth", "status"]);
    assert_eq!(code(&o), 0, "stderr: {}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("signed in as sargbench2 (member) since 2026-08-20"), "{out}");
    assert!(out.contains("token from flag"), "{out}");
    assert!(out.contains("called by unknown"), "no agent markers in a test env: {out}");
    assert!(!out.contains("tok-123"), "token must never be printed: {out}");

    let o = sb.sarg_in(&["--json", "auth", "status"]);
    assert_eq!(code(&o), 0);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["handle"], "sargbench2");
    assert_eq!(v["signed_in"], true);
    assert_eq!(v["token_source"], "flag");
    assert_eq!(v["agent"], "unknown");
    m.assert_hits(2);
}

#[test]
fn agent_is_detected_from_claude_markers_or_sarg_agent() {
    let sb = Sandbox::new();
    sb.json_get("/me", r#"{"handle":"sargbench2","role":"member","created":"2026-08-20T14:52:01+00:00"}"#);
    let o = sb.sarg_env(
        &["--token", "t", "--json", "auth", "status"],
        &[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "abc-123")],
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["agent"], "claude-code");
    assert_eq!(v["session"], "abc-123");

    let o = sb.sarg_env(
        &["--token", "t", "--json", "auth", "status"],
        &[("AI_AGENT", "claude-code_2-1-259_agent")],
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["agent"], "claude-code");

    // The explicit override wins over every marker.
    let o = sb.sarg_env(
        &["--token", "t", "--json", "auth", "status"],
        &[("CLAUDECODE", "1"), ("SARG_AGENT", "codex"), ("SARG_SESSION", "s9")],
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["agent"], "codex");
    assert_eq!(v["session"], "s9");
}

#[test]
fn token_is_never_read_from_the_environment_or_claude_settings() {
    let sb = Sandbox::new();
    sb.json_get("/", r#"{"public_lessons":701,"public_models":129}"#);
    let claude = sb.home.join(".claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(
        claude.join("settings.json"),
        r#"{"env":{"SARGINEER_WEB_TOKEN":"from-claude"}}"#,
    )
    .unwrap();
    let o = sb.sarg_env(&["auth", "status"], &[("SARGINEER_WEB_TOKEN", "from-env")]);
    assert_eq!(code(&o), 3, "must be signed out: {}", stdout(&o));
    assert!(stdout(&o).contains("not signed in"));
}

#[test]
fn auth_status_without_token_exits_3_with_public_counts() {
    let sb = Sandbox::new();
    sb.json_get("/", r#"{"public_lessons":701,"public_models":129}"#);
    let o = sb.sarg(&["auth", "status"]);
    assert_eq!(code(&o), 3, "stdout: {} stderr: {}", stdout(&o), stderr(&o));
    assert!(stdout(&o).contains("701 lessons"), "{}", stdout(&o));
    assert!(stdout(&o).contains("not signed in"));
    assert!(stdout(&o).contains("sarg config set token"));
}

#[test]
fn rejected_token_maps_401_to_exit_3_and_passes_hint() {
    let sb = Sandbox::new();
    sb.server.mock(|when, then| {
        when.method(GET).path("/me");
        then.status(401)
            .header("content-type", "application/json")
            .body(r#"{"error":"not signed in","hint":"POST /login"}"#);
    });
    let o = sb.sarg(&["--token", "bad", "auth", "status"]);
    assert_eq!(code(&o), 3);
    let err = stderr(&o);
    assert!(err.contains("not signed in"), "{err}");
    assert!(err.contains("hint: POST /login"), "{err}");
}

#[test]
fn api_is_cached_for_a_day_and_refresh_bypasses() {
    let sb = Sandbox::new();
    let m = sb.json_get("/api", API_FIXTURE);
    let o = sb.sarg(&["api"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("sargineer v1.12.0"), "{out}");
    assert!(out.contains("fetched now"), "{out}");
    assert!(out.contains("POST   /notes"), "{out}");

    let o = sb.sarg(&["api"]);
    assert!(stdout(&o).contains("cached"), "{}", stdout(&o));
    m.assert_hits(1);
    assert!(sb.home.join(".sargineer/cache/api.json").exists());

    let o = sb.sarg(&["api", "--refresh"]);
    assert!(stdout(&o).contains("fetched now"));
    m.assert_hits(2);

    let o = sb.sarg(&["--json", "api", "--fields"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["required"][0], "title");
    assert!(v["status"].as_array().unwrap().iter().any(|s| s == "working"));
    m.assert_hits(2);

    // The first look records the version; nothing is announced.
    let cfg = fs::read_to_string(sb.config_path()).unwrap();
    assert!(cfg.contains("last_seen_version = \"v1.12.0\""), "{cfg}");
}

#[test]
fn version_drift_is_announced_once_and_changelog_defaults_to_it() {
    let sb = Sandbox::new();
    fs::create_dir_all(sb.config_path().parent().unwrap()).unwrap();
    fs::write(sb.config_path(), "last_seen_version = \"v1.11.0\"\n").unwrap();
    sb.json_get("/api", API_FIXTURE);
    let cl = sb.server.mock(|when, then| {
        when.method(GET)
            .path("/changelog")
            .query_param("since", "v1.11.0");
        then.status(200)
            .header("content-type", "application/json")
            .body(r###"{"version":"v1.12.0","entries":[{"date":"2026-08-28","md":"## 2026-08-28 — v1.12.0 — file roles","version":"v1.12.0"}]}"###);
    });
    let o = sb.sarg(&["api"]);
    assert!(stderr(&o).contains("moved v1.11.0 → v1.12.0"), "{}", stderr(&o));
    let o = sb.sarg(&["api"]);
    assert!(!stderr(&o).contains("moved"), "announced twice: {}", stderr(&o));

    let o = sb.sarg(&["changelog"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("file roles"), "{}", stdout(&o));
    cl.assert_hits(1);
}

#[test]
fn raw_get_prints_body_and_maps_status() {
    let sb = Sandbox::new();
    sb.server.mock(|when, then| {
        when.method(GET).path("/queue");
        then.status(200)
            .header("content-type", "text/markdown")
            .body("# queue/ — what to make next\n");
    });
    sb.server.mock(|when, then| {
        when.method(POST).path("/notes").body_contains("\"title\"");
        then.status(400)
            .header("content-type", "application/json")
            .body(r#"{"error":"missing fields","missing":["symptom"],"hint":"GET /api note_fields"}"#);
    });
    let o = sb.sarg(&["raw", "GET", "/queue"]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).starts_with("# queue/"));

    let o = sb.sarg_in(&["raw", "POST", "/notes", "-d", r#"{"title":"x"}"#]);
    assert_eq!(code(&o), 4, "validation errors exit 4: {}", stderr(&o));
    assert!(stderr(&o).contains("symptom"), "{}", stderr(&o));
    assert!(stderr(&o).contains("hint: GET /api note_fields"));

    let o = sb.sarg(&["raw", "GET", "queue"]);
    assert_eq!(code(&o), 2, "usage errors exit 2");
}

#[test]
fn init_persists_the_flag_token_in_sargineer_dir_with_mode_0600() {
    let sb = Sandbox::new();
    sb.server.mock(|when, then| {
        when.method(GET).path("/me").header("authorization", "Bearer secret-xyz");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"handle":"sargbench2","role":"member"}"#);
    });
    let o = sb.sarg(&["--token", "secret-xyz", "init"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("signed in as sargbench2"), "{out}");
    assert!(out.contains("changed      url, token, host_tags"), "{out}");
    assert!(!out.contains("secret-xyz"));

    let p = sb.config_path();
    assert!(p.exists(), "config must live under ~/.sargineer");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    }
    let cfg = fs::read_to_string(&p).unwrap();
    assert!(cfg.contains("token = \"secret-xyz\""), "{cfg}");
    assert!(cfg.contains("host_tags = ["), "{cfg}");
    assert!(cfg.contains(&format!("\"{}\"", std::env::consts::OS)), "{cfg}");
    assert!(sb.home.join(".sargineer/pending").is_dir());
    assert!(sb.home.join(".sargineer/cache").is_dir());

    // With the token persisted, later calls need no flag at all.
    let o = sb.sarg(&["auth", "status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("token from ~/.sargineer/config.toml"), "{}", stdout(&o));
}

#[test]
fn init_without_a_token_says_what_to_do_next() {
    let sb = Sandbox::new();
    let o = sb.sarg(&["init", "--offline", "--no-probe"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("next: sarg config set token"), "{}", stdout(&o));
    assert!(sb.config_path().exists());
}

#[test]
fn config_get_masks_token_and_set_roundtrips() {
    let sb = Sandbox::new();
    let o = sb.sarg(&["config", "set", "token", "abcdefgh1234"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let o = sb.sarg(&["config", "set", "deny_terms", "acme, widgetco"]);
    assert_eq!(code(&o), 0);
    let o = sb.sarg(&["config", "set", "project_aliases.hat", "pi-hat-analog-out"]);
    assert_eq!(code(&o), 0);

    let o = sb.sarg(&["config", "get"]);
    let out = stdout(&o);
    assert!(out.contains("****1234"), "{out}");
    assert!(!out.contains("abcdefgh1234"));
    assert!(out.contains("acme, widgetco"), "{out}");
    assert!(out.contains("alias.hat"), "{out}");

    let o = sb.sarg(&["config", "get", "token", "--reveal"]);
    assert_eq!(stdout(&o).trim(), "abcdefgh1234");

    let o = sb.sarg(&["config", "get", "deny_terms"]);
    assert_eq!(stdout(&o).trim(), "acme,widgetco");

    let o = sb.sarg(&["config", "set", "bogus", "1"]);
    assert_eq!(code(&o), 2);

    let o = sb.sarg(&["config", "path"]);
    assert_eq!(Path::new(stdout(&o).trim()), sb.config_path());
}

#[test]
fn doc_prints_server_markdown() {
    let sb = Sandbox::new();
    sb.server.mock(|when, then| {
        when.method(GET).path("/share.md");
        then.status(200)
            .header("content-type", "text/markdown")
            .body("# Saving what you learn\n");
    });
    let o = sb.sarg(&["doc", "share"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o).trim(), "# Saving what you learn");
}

#[test]
fn unreachable_server_exits_5() {
    let sb = Sandbox::new();
    let o = sb.sarg(&["--url", "http://127.0.0.1:9", "--token", "t", "auth", "status"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("cannot reach"));
}
