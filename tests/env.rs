//! `sarg env`: a named dev server beside prod, with its own token, cache
//! and spool, switched in config.toml so hooks follow. The mock server
//! plays the dev instance; prod stays the default sargineer.com and is
//! never contacted here.

mod common;

use std::fs;

use common::*;
use httpmock::prelude::*;

const API_FIXTURE: &str = include_str!("fixtures/api.json");

fn me_mock<'a>(sb: &'a Sandbox, token: &str) -> httpmock::Mock<'a> {
    let auth = format!("Bearer {token}");
    sb.server.mock(move |when, then| {
        when.method(GET).path("/me").header("authorization", &auth);
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"handle":"dev","role":"admin","created":"2026-08-26T00:00:00+00:00"}"#);
    })
}

#[test]
fn define_switch_show_and_remove() {
    let sb = Sandbox::new();
    let url = sb.server.base_url();

    // Define + switch in one go. The token is stored, never echoed.
    let o = sb.sarg_bare(&["env", "dev", "--url", &url, "--token", "tok-dev"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains("dev") && out.contains(&url) && out.contains("now active"),
        "{out}"
    );
    assert!(!out.contains("tok-dev"), "token printed: {out}");
    let cfg = fs::read_to_string(sb.config_path()).unwrap();
    assert!(cfg.contains("env = \"dev\""), "{cfg}");
    assert!(cfg.contains("[envs.dev]"), "{cfg}");
    assert!(
        cfg.contains("tok-dev"),
        "token must be stored in config.toml: {cfg}"
    );

    // Listing marks the active one; prod is always there.
    let o = sb.sarg_bare(&["env"]);
    let out = stdout(&o);
    assert!(
        out.contains("* dev") || out.contains("*\u{1b}"),
        "active marker: {out}"
    );
    assert!(
        out.contains("prod") && out.contains("sargineer.com"),
        "{out}"
    );
    assert!(
        out.contains("no token"),
        "prod has no token in this sandbox: {out}"
    );
    assert!(!out.contains("tok-dev"), "{out}");

    let o = sb.sarg_bare(&["--json", "env"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["active"], "dev");
    assert_eq!(v["envs"][1]["name"], "dev");
    assert_eq!(v["envs"][1]["token"], true);
    assert_eq!(v["envs"][1]["token_masked"], "****");

    // Back to prod, then remove.
    let o = sb.sarg_bare(&["env", "prod"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let cfg = fs::read_to_string(sb.config_path()).unwrap();
    assert!(!cfg.contains("env = \"dev\""), "{cfg}");
    assert!(
        cfg.contains("[envs.dev]"),
        "definition survives a switch: {cfg}"
    );

    let o = sb.sarg_bare(&["env", "dev", "--rm"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let cfg = fs::read_to_string(sb.config_path()).unwrap();
    assert!(!cfg.contains("[envs.dev]"), "{cfg}");

    // An unknown env is a usage error naming the fix; nothing is contacted.
    let o = sb.sarg_bare(&["env", "staging"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("--url"), "{}", stderr(&o));
    let o = sb.sarg_bare(&["--env", "nope", "auth", "status"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("unknown env `nope`"), "{}", stderr(&o));
}

#[test]
fn active_env_routes_calls_with_its_own_token_and_says_so() {
    let sb = Sandbox::new();
    let url = sb.server.base_url();
    let me = me_mock(&sb, "tok-dev");
    sb.sarg_bare(&["env", "dev", "--url", &url, "--token", "tok-dev"]);

    // No --url, no --token: the active env supplies both.
    let o = sb.sarg_bare(&["auth", "status"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("signed in as dev"), "{}", stdout(&o));
    assert!(
        stdout(&o).contains("token from ~/.sargineer/config.toml"),
        "{}",
        stdout(&o)
    );
    assert!(
        stderr(&o).contains("sarg: env dev"),
        "banner on stderr: {}",
        stderr(&o)
    );
    me.assert_hits(1);

    // A one-off --env prod ignores the dev token and prints no banner.
    // (prod is pointed at the mock too, so nothing leaves the sandbox.)
    let api = sb.json_get("/api", API_FIXTURE);
    let o = sb.sarg_bare(&["env", "prod", "--url", &url, "--no-switch"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let o = sb.sarg_bare(&["--env", "prod", "api"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!stderr(&o).contains("sarg: env"), "{}", stderr(&o));
    api.assert_hits(1);
    assert!(
        sb.home.join(".sargineer/cache/api.json").is_file(),
        "prod cache"
    );
    assert!(
        !sb.home.join(".sargineer/envs/dev").exists(),
        "dev untouched"
    );
    me.assert_hits(1);
    let cfg = fs::read_to_string(sb.config_path()).unwrap();
    assert!(
        cfg.contains("env = \"dev\""),
        "--no-switch left dev active: {cfg}"
    );

    // doctor reports the env as a problem, so nobody mistakes dev for prod.
    let o = sb.sarg_bare(&["--json", "doctor"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["env"], "dev");
    assert!(
        v["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("env dev")),
        "{v}"
    );
}

#[test]
fn env_keeps_its_own_cache_spool_and_version() {
    let sb = Sandbox::new();
    let url = sb.server.base_url();
    let api = sb.json_get("/api", API_FIXTURE);
    sb.sarg_bare(&["env", "dev", "--url", &url, "--token", "tok-dev"]);

    let o = sb.sarg_bare(&["api"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    api.assert_hits(1);
    assert!(sb.home.join(".sargineer/envs/dev/cache/api.json").is_file());
    assert!(
        !sb.home.join(".sargineer/cache/api.json").exists(),
        "prod cache untouched"
    );

    // The version seen lands on the env entry, not on prod's slot.
    let cfg = fs::read_to_string(sb.config_path()).unwrap();
    let head = cfg.split("[envs").next().unwrap();
    assert!(
        !head.contains("last_seen_version"),
        "prod slot must stay empty: {cfg}"
    );
    assert!(cfg.contains("last_seen_version"), "{cfg}");

    // A note that cannot be sent (here: the env has no token yet) waits
    // under the env, so a later `sarg sync` on prod cannot send it there.
    let o = sb.sarg_bare(&["env", "tokenless", "--url", &url]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let o = sb.sarg_bare(&[
        "lesson",
        "new",
        "--title",
        "Spooled while signed out of the env",
        "--symptom",
        "s",
        "--cause",
        "c",
        "--fix",
        "f",
        "--step",
        "do it",
        "--check",
        "it works",
        "--intent",
        "test",
        "--project",
        "bench",
        "--hw",
        "esp32",
    ]);
    assert_eq!(code(&o), 5, "offline exit: {}", stderr(&o));
    let spooled = fs::read_dir(sb.home.join(".sargineer/envs/tokenless/pending"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(spooled, 1, "{}", stderr(&o));
    assert!(
        !sb.home.join(".sargineer/pending").exists(),
        "prod spool untouched"
    );
    let o = sb.sarg_bare(&["env", "prod"]);
    assert_eq!(code(&o), 0);
    let o = sb.sarg_bare(&["sync"]);
    assert!(stdout(&o).contains("nothing spooled"), "{}", stdout(&o));
}

#[test]
fn config_keys_cover_env_and_envs() {
    let sb = Sandbox::new();
    let o = sb.sarg(&["config", "set", "envs.dev.url", "http://127.0.0.1:8093/"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let o = sb.sarg(&["config", "set", "env", "dev"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let o = sb.sarg(&["config", "set", "env", "missing"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    let o = sb.sarg(&["config", "unset", "envs.dev"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let cfg = fs::read_to_string(sb.config_path()).unwrap();
    assert!(
        !cfg.contains("env ="),
        "removing the active env resets it: {cfg}"
    );
}
