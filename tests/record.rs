//! Recording verbs and the offline spool, against a mock sargineer.

mod common;

use std::fs;

use common::*;
use httpmock::prelude::*;

const API_FIXTURE: &str = include_str!("fixtures/api.json");

/// Every record test needs /api (the validator reads note_fields).
fn with_api(sb: &Sandbox) {
    sb.json_get("/api", API_FIXTURE);
}

#[test]
fn lesson_new_validates_scans_and_posts_private() {
    let sb = Sandbox::new();
    with_api(&sb);
    let post = sb.server.mock(|when, then| {
        when.method(POST)
            .path("/notes")
            .header("authorization", "Bearer tok-123")
            .body_contains("esptool");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"esptool-write-flash-fails-at-0x0","handle":"sargbench2","visibility":"private"}"#);
    });
    let o = sb.sarg_in(&[
        "lesson",
        "new",
        "--title",
        "esptool write-flash fails at 0x0 on esp32-s3 error 0x05",
        "--symptom",
        "A fatal error occurred: Packet content transfer stopped",
        "--fix",
        "erase-flash first, then write at 0x0",
        "--step",
        "esptool erase-flash",
        "--step",
        "esptool write-flash 0x0 app.bin",
        "--check",
        "chip boots",
        "--intent",
        "flash an esp32-s3",
        "--project",
        "bench",
        "--hw",
        "esp32-s3",
    ]);
    assert_eq!(code(&o), 0, "stderr: {}", stderr(&o));
    assert!(stdout(&o).contains("saved private"), "{}", stdout(&o));
    assert!(
        stdout(&o).contains("sargbench2/esptool-write-flash-fails-at-0x0"),
        "{}",
        stdout(&o)
    );
    post.assert_hits(1);
}

#[test]
fn lesson_new_attaches_host_tags_and_project_alias() {
    let sb = Sandbox::new();
    with_api(&sb);
    // seed config with host tags + an alias
    sb.sarg(&["config", "set", "host_tags", "linux,x86_64"]);
    sb.sarg(&["config", "set", "project_aliases.hat", "pi-hat-analog-out"]);
    let post = sb.server.mock(|when, then| {
        when.method(POST)
            .path("/notes")
            .body_contains("pi-hat-analog-out")
            .body_contains("x86_64");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"x","handle":"me"}"#);
    });
    let o = sb.sarg_in(&[
        "lesson",
        "new",
        "--title",
        "DAC8760 needs a 0.1uF cap or it oscillates at 0x7f",
        "--symptom",
        "output rings",
        "--fix",
        "add the cap",
        "--intent",
        "bring up an analog output",
        "--project",
        "hat",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    post.assert_hits(1);
}

#[test]
fn lesson_new_refuses_a_deny_term_and_does_not_post() {
    let sb = Sandbox::new();
    with_api(&sb);
    sb.sarg(&["config", "set", "deny_terms", "acme,zeta"]);
    let post = sb.server.mock(|when, then| {
        when.method(POST).path("/notes");
        then.status(200).body("{}");
    });
    let o = sb.sarg_in(&[
        "lesson",
        "new",
        "--title",
        "The acme hat needs a pull-up on reset line 0x1",
        "--symptom",
        "reset floats",
        "--fix",
        "add 10k",
        "--intent",
        "bring up a hat",
        "--project",
        "hat",
    ]);
    assert_eq!(
        code(&o),
        4,
        "deny term must block with exit 4: {}",
        stdout(&o)
    );
    assert!(stderr(&o).contains("deny"), "{}", stderr(&o));
    assert!(stderr(&o).contains("can never be sent"), "{}", stderr(&o));
    post.assert_hits(0);
}

#[test]
fn lesson_new_blocks_pii_unless_allowed() {
    let sb = Sandbox::new();
    with_api(&sb);
    let post = sb.server.mock(|when, then| {
        when.method(POST).path("/notes");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"x","handle":"me"}"#);
    });
    let base = [
        "lesson",
        "new",
        "--title",
        "board at 192.168.86.26 drops wifi at 0x0",
        "--symptom",
        "it disconnects",
        "--fix",
        "pin the channel",
        "--intent",
        "keep a board online",
        "--project",
        "bench",
    ];
    let o = sb.sarg_in(&base);
    assert_eq!(code(&o), 4, "private IP should block: {}", stderr(&o));
    assert!(stderr(&o).contains("--allow-pii"), "{}", stderr(&o));
    post.assert_hits(0);

    let mut with_flag = base.to_vec();
    with_flag.push("--allow-pii");
    let o = sb.sarg_in(&with_flag);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    post.assert_hits(1);
}

#[test]
fn lesson_new_rejects_placeholder_title() {
    let sb = Sandbox::new();
    with_api(&sb);
    let o = sb.sarg_in(&[
        "lesson",
        "new",
        "--title",
        "Fix for <exact error> on <hw>",
        "--symptom",
        "x",
        "--fix",
        "y",
        "--intent",
        "test",
        "--project",
        "bench",
    ]);
    assert_eq!(code(&o), 4);
    assert!(stderr(&o).contains("placeholder"), "{}", stderr(&o));
}

#[test]
fn dry_run_validates_without_posting() {
    let sb = Sandbox::new();
    with_api(&sb);
    let post = sb.server.mock(|when, then| {
        when.method(POST).path("/notes");
        then.status(200).body("{}");
    });
    let o = sb.sarg_in(&[
        "lesson",
        "new",
        "--dry-run",
        "--title",
        "esptool chip-id returns 0x0 on a dead board",
        "--symptom",
        "reads zero",
        "--fix",
        "replace the board",
        "--intent",
        "identify a board",
        "--project",
        "bench",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("\"title\""), "{}", stdout(&o));
    post.assert_hits(0);
}

#[test]
fn missing_token_spools_and_sync_uploads() {
    let sb = Sandbox::new();
    with_api(&sb);
    // No token → spool. (config has no token; no --token flag)
    let o = sb.sarg(&[
        "lesson",
        "new",
        "--title",
        "openmv n6 csi init fails below HD at 0x0",
        "--symptom",
        "init error",
        "--fix",
        "use HD",
        "--intent",
        "capture bayer frames",
        "--project",
        "bench",
    ]);
    assert_eq!(
        code(&o),
        5,
        "no token should spool (exit 5): {}",
        stderr(&o)
    );
    let pending: Vec<_> = fs::read_dir(sb.home.join(".sargineer/pending"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(pending.len(), 1, "one spooled note");

    // Now sync with a token.
    let post = sb.server.mock(|when, then| {
        when.method(POST)
            .path("/notes")
            .body_contains("openmv n6 csi");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"openmv","handle":"sargbench2"}"#);
    });
    let o = sb.sarg_in(&["sync"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("1 sent"), "{}", stdout(&o));
    post.assert_hits(1);
    let left: Vec<_> = fs::read_dir(sb.home.join(".sargineer/pending"))
        .unwrap()
        .flatten()
        .collect();
    assert!(left.is_empty(), "spool cleared after sync");
}

#[test]
fn issue_new_sets_kind_and_about() {
    let sb = Sandbox::new();
    with_api(&sb);
    let post = sb.server.mock(|when, then| {
        when.method(POST)
            .path("/notes")
            .body_contains("\"kind\":\"issue\"")
            .body_contains("sargineer.com");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"i","handle":"me"}"#);
    });
    let o = sb.sarg_in(&[
        "issue",
        "--about",
        "sargineer.com",
        "--title",
        "publish returns an empty 303 so a client cannot tell it worked",
        "--symptom",
        "empty body on publish",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    post.assert_hits(1);
}

#[test]
fn publish_needs_authorization_from_an_agent() {
    let sb = Sandbox::new();
    let publish = sb.server.mock(|when, then| {
        when.method(POST).path("/n/sargbench2/x/publish");
        then.status(303).body("");
    });
    // Non-TTY (test) without the flag → refused, nothing sent.
    let o = sb.sarg_in(&["lesson", "publish", "sargbench2/x"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("user's own act"), "{}", stderr(&o));
    publish.assert_hits(0);

    // With --user-authorized → publishes, then GETs to echo state.
    let after = sb.json_get(
        "/n/sargbench2/x",
        r#"{"id":"x","handle":"sargbench2","visibility":"public"}"#,
    );
    let o = sb.sarg_in(&["lesson", "publish", "sargbench2/x", "--user-authorized"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("is now public"), "{}", stdout(&o));
    publish.assert_hits(1);
    after.assert_hits(1);
}

#[test]
fn edit_gates_pii_the_same_way_as_new() {
    let sb = Sandbox::new();
    sb.json_get("/api", include_str!("fixtures/api.json"));
    sb.json_get("/n/sargbench2/x", include_str!("fixtures/note.json"));
    let put = sb.server.mock(|when, then| {
        when.method(PUT).path("/n/sargbench2/x");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"ok":true}"#);
    });
    // A scripted $EDITOR that replaces the note with one carrying an email.
    let editor = sb.home.join("fake-editor.sh");
    fs::write(
        &editor,
        concat!(
            "#!/bin/sh\n",
            "cat > \"$1\" <<'NOTE'\n",
            "kind = \"lesson\"\n",
            "title = \"esptool write-flash fails at 0x0 on esp32-s3 with error -71\"\n",
            "symptom = \"error -71\"\n",
            "cause = \"unpowered hub\"\n",
            "fix = \"rear port; ask support@vendorx.com.\"\n",
            "intent = \"flash a heltec v4\"\n",
            "project = \"meshbase\"\n",
            "status = \"working\"\n",
            "NOTE\n",
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    let env = [("EDITOR", editor.to_str().unwrap())];

    let o = sb.sarg_env(&["--token", "t", "lesson", "edit", "sargbench2/x"], &env);
    assert_eq!(code(&o), 4, "email blocks: {}", stderr(&o));
    assert!(
        stderr(&o).contains("support@vendorx.com") && stderr(&o).contains("--allow-pii"),
        "{}",
        stderr(&o)
    );
    put.assert_hits(0);

    let o = sb.sarg_env(
        &[
            "--token",
            "t",
            "lesson",
            "edit",
            "sargbench2/x",
            "--allow-pii",
        ],
        &env,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stderr(&o).contains("sending email"), "{}", stderr(&o));
    put.assert_hits(1);
}

#[test]
fn lesson_new_survives_large_numbers_in_text() {
    // "256000 baud" once overflowed the private-IP scanner's octet accumulator.
    let sb = Sandbox::new();
    with_api(&sb);
    let post = sb.server.mock(|when, then| {
        when.method(POST).path("/notes").body_contains("256000");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"x","handle":"me"}"#);
    });
    let o = sb.sarg_in(&[
        "lesson",
        "new",
        "--title",
        "Radar UART drops bytes at 256000 baud unless the rx buffer is 0x400",
        "--symptom",
        "frames truncated at 4294967296 bytes",
        "--fix",
        "set the buffer",
        "--intent",
        "read the radar",
        "--project",
        "bench",
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    post.assert_hits(1);
}
