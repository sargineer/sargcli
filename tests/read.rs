//! The read-side verbs against recorded sargineer responses (v1.12.0).

mod common;

use std::fs;

use common::*;
use httpmock::prelude::*;

const NOTES_FULL: &str = include_str!("fixtures/notes_full.json");
const SEARCH: &str = include_str!("fixtures/search.json");
const NOTE: &str = include_str!("fixtures/note.json");
const PART: &str = include_str!("fixtures/part.json");
const MODELS: &str = include_str!("fixtures/models.json");
const MODEL: &str = include_str!("fixtures/model.json");
const TAG: &str = include_str!("fixtures/tag.json");

#[test]
fn ask_merges_search_and_notes_and_ranks_by_status() {
    let sb = Sandbox::new();
    let search = sb.server.mock(|when, then| {
        when.method(GET).path("/search").query_param("q", "heltec v4");
        then.status(200).header("content-type", "application/json").body(SEARCH);
    });
    let notes = sb.server.mock(|when, then| {
        when.method(GET)
            .path("/notes")
            .query_param("q", "heltec v4")
            .query_param("full", "1")
            .query_param("n", "8");
        then.status(200).header("content-type", "application/json").body(NOTES_FULL);
    });
    let o = sb.sarg_in(&["ask", "heltec", "v4"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("sarg \"heltec v4\" · 3 lessons + 5 title-only · 111 match in all · 1 part"), "{out}");
    assert!(out.contains("title only — sarg show for the rest"), "{out}");
    assert!(out.contains("part  heltec-wifi-lora-32-v4"), "{out}");
    assert!(out.contains(" 1. [working] "), "{out}");
    assert!(out.contains("[working] sarg/flash-meshcore-onto-heltec-wifi-lora-32-v4"), "{out}");
    assert!(out.contains("fix: curl flasher.meshcore.io/releases"), "{out}");
    assert!(out.contains("next: sarg show "), "{out}");
    // Title-only hits from /search sort after the full lessons.
    let pos_full = out.find("[working] sarg/flash-meshcore").unwrap();
    let pos_title = out.find("title only").unwrap();
    assert!(pos_full < pos_title, "{out}");
    search.assert_hits(1);
    notes.assert_hits(1);

    // Machine mode: the merged structure.
    let o = sb.sarg_in(&["--json", "ask", "heltec", "v4"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["total"], 111);
    assert_eq!(v["notes"].as_array().unwrap().len(), 8);
    assert_eq!(v["parts"].as_array().unwrap().len(), 1);
    assert_eq!(v["notes"][0]["status"], "working");
    assert_eq!(v["notes"][7]["title_only"], true);
}

#[test]
fn ask_names_the_gap_when_nothing_matches() {
    let sb = Sandbox::new();
    sb.json_get("/search", r#"{"q":"zorbtron","parts":[],"notes":[]}"#);
    sb.json_get("/notes", r#"{"count":0,"total":111,"results":[]}"#);
    let o = sb.sarg_in(&["ask", "zorbtron"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("no lessons about \"zorbtron\""), "{}", stdout(&o));
}

#[test]
fn ask_survives_a_failing_search_endpoint() {
    let sb = Sandbox::new();
    sb.server.mock(|when, then| {
        when.method(GET).path("/search");
        then.status(500).body("boom");
    });
    sb.json_get("/notes", NOTES_FULL);
    let o = sb.sarg_in(&["ask", "heltec"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("3 lessons · 111 match in all · 0 parts"), "{}", stdout(&o));
}

#[test]
fn ask_signed_out_warns_that_fixes_are_locked() {
    let sb = Sandbox::new();
    sb.json_get("/search", SEARCH);
    sb.json_get("/notes", NOTES_FULL);
    let o = sb.sarg(&["ask", "heltec"]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).contains("signed out: fixes and steps are locked"), "{}", stdout(&o));
}

#[test]
fn show_renders_every_field_and_fences_unverified() {
    let sb = Sandbox::new();
    sb.json_get("/n/sargbench2/heltec-v4-running-meshcore-companion-radio-usb-never", NOTE);
    let o = sb.sarg_in(&["show", "sargbench2/heltec-v4-running-meshcore-companion-radio-usb-never"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("[working] sargbench2/heltec-v4-running"), "{out}");
    for label in ["setup", "symptom", "cause", "fix", "steps", "check"] {
        assert!(out.contains(&format!("\n{label}\n")), "missing {label}: {out}");
    }
    assert!(out.contains("   1. Unplug the board"), "{out}");
    assert!(out.contains("unverified — nobody confirmed these; never present them as the fix"), "{out}");
    assert!(out.contains("  ? No current measurement"), "{out}");
    assert!(out.contains("project: mesh-basestation"), "{out}");
    assert!(out.contains("host: linux, x86_64, ubuntu-24"), "{out}");
}

#[test]
fn show_bare_id_is_looked_up() {
    let sb = Sandbox::new();
    sb.json_get("/notes", NOTES_FULL);
    let m = sb.json_get("/n/sarg/flash-meshcore-onto-heltec-wifi-lora-32-v4", NOTE);
    let o = sb.sarg_in(&["show", "flash-meshcore-onto-heltec-wifi-lora-32-v4"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    m.assert_hits(1);

    let o = sb.sarg_in(&["show", "no-such-id"]);
    assert_eq!(code(&o), 2);
}

#[test]
fn part_surfaces_facts_models_lessons_and_estimate_flags() {
    let sb = Sandbox::new();
    sb.json_get("/p/heltec-wifi-lora-32-v4", PART);
    let o = sb.sarg_in(&["part", "heltec-wifi-lora-32-v4"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("heltec-wifi-lora-32-v4 · Heltec WiFi LoRa 32 V4"), "{out}");
    assert!(out.contains("owned ×2"), "{out}");
    assert!(out.contains("size_mm     [51.7,25.4,10.7]") || out.contains("size_mm     [\n") || out.contains("size_mm"), "{out}");
    assert!(out.contains("model       sarg/heltec-wifi-lora-32-v4 · envelope · verified"), "{out}");
    assert!(out.contains("amz-heltec-lora-v4.step"), "{out}");
    assert!(out.contains("facts partly estimated"), "{out}");
    assert!(out.contains("bring-up"), "{out}");
    assert!(out.contains("lessons 51 about this part"), "{out}");
    assert!(out.contains("sargbench2/heltec-v4-running-meshcore-companion-radio-usb-never"), "{out}");
}

#[test]
fn part_404_exits_1_with_server_message() {
    let sb = Sandbox::new();
    sb.server.mock(|when, then| {
        when.method(GET).path("/p/nope");
        then.status(404).header("content-type", "application/json").body(r#"{"error":"no such part"}"#);
    });
    let o = sb.sarg_in(&["part", "nope"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("no such part"));
}

#[test]
fn tag_and_vendor_list_parts_and_notes() {
    let sb = Sandbox::new();
    sb.json_get("/t/heltec-v4", TAG);
    let o = sb.sarg_in(&["tag", "heltec-v4", "-n", "2"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("tag heltec-v4 · 2 parts · 4 lessons"), "{out}");
    assert!(out.contains("… 2 more · -n 4"), "{out}");

    sb.json_get("/v/heltec", TAG);
    let o = sb.sarg_in(&["vendor", "heltec"]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).starts_with("vendor heltec"));
}

#[test]
fn id_names_the_chip_and_finds_lessons() {
    let sb = Sandbox::new();
    let notes = sb.server.mock(|when, then| {
        when.method(GET).path("/notes").query_param("q", "303a:1001");
        then.status(200).header("content-type", "application/json").body(NOTES_FULL);
    });
    let o = sb.sarg_in(&["id", "303a:1001"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("303a:1001"), "{out}");
    assert!(out.contains("Espressif — USB JTAG/serial"), "{out}");
    assert!(out.contains("sarg/flash-meshcore-onto-heltec-wifi-lora-32-v4"), "{out}");
    notes.assert_hits(1);

    sb.json_get("/notes", r#"{"count":0,"total":0,"results":[]}"#);
    let o = sb.sarg_in(&["id", "dead:beef"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("unknown vendor"), "{}", stdout(&o));
    assert!(stdout(&o).contains("no lessons mention dead:beef"), "{}", stdout(&o));

    let o = sb.sarg_in(&["id", "garbage"]);
    assert_eq!(code(&o), 2);
}

#[cfg(windows)]
#[test]
fn id_on_windows_explains_how_to_find_a_usb_id() {
    let sb = Sandbox::new();

    let o = sb.sarg(&["id"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("Device Manager"), "{}", stderr(&o));
    assert!(stderr(&o).contains("USB\\VID_xxxx&PID_yyyy"), "{}", stderr(&o));

    let o = sb.sarg(&["id", "COM3"]);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("serial-port lookup"), "{}", stderr(&o));
}

#[test]
fn preflight_reports_each_board_and_the_gaps_and_writes_nothing() {
    let sb = Sandbox::new();
    sb.server.mock(|when, then| {
        when.method(GET).path("/search").query_param("q", "heltec v4");
        then.status(200).header("content-type", "application/json").body(SEARCH);
    });
    sb.server.mock(|when, then| {
        when.method(GET).path("/notes").query_param("q", "heltec v4");
        then.status(200).header("content-type", "application/json").body(NOTES_FULL);
    });
    sb.server.mock(|when, then| {
        when.method(GET).path("/search").query_param("q", "flash over dfu");
        then.status(200).header("content-type", "application/json").body(r#"{"q":"flash over dfu","parts":[],"notes":[]}"#);
    });
    sb.server.mock(|when, then| {
        when.method(GET).path("/notes").query_param("q", "flash over dfu");
        then.status(200).header("content-type", "application/json").body(r#"{"count":0,"total":111,"results":[]}"#);
    });
    sb.json_get("/parts", r#"{"count":0,"results":[]}"#);
    let o = sb.sarg_in(&["preflight", "heltec v4", "--intent", "flash over dfu"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("live from sargineer"), "{out}");
    assert!(out.contains("board heltec v4"), "{out}");
    assert!(out.contains("intent flash over dfu"), "{out}");
    assert!(out.contains("gaps: \"flash over dfu\""), "{out}");
    let leftovers: Vec<_> = fs::read_dir(&sb.home)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_lowercase().contains("preflight"))
        .collect();
    assert!(leftovers.is_empty(), "preflight must not persist: {leftovers:?}");
}

#[test]
fn cad_ls_groups_by_role_and_get_downloads_bytes() {
    let sb = Sandbox::new();
    sb.json_get("/models/heltec-wifi-lora-32-v4", MODELS);
    sb.json_get("/m/sarg/heltec-wifi-lora-32-v4", MODEL);
    let step = sb.server.mock(|when, then| {
        when.method(GET)
            .path("/m/sarg/heltec-wifi-lora-32-v4/f/amz-heltec-lora-v4.step")
            .header("authorization", "Bearer tok-123");
        then.status(200)
            .header("content-type", "application/octet-stream")
            .body(vec![0u8, 1, 2, 3, 255]);
    });

    let o = sb.sarg_in(&["cad", "ls", "heltec-wifi-lora-32-v4"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("1 model"), "{out}");
    assert!(out.contains("CAD geometry"), "{out}");
    assert!(out.contains("amz-heltec-lora-v4.step"), "{out}");
    assert!(out.contains("agent source"), "{out}");

    let o = sb.sarg_in(&["cad", "get", "sarg/heltec-wifi-lora-32-v4", "--role", "cad", "-o", "dl"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let f = sb.home.join("dl/amz-heltec-lora-v4.step");
    assert_eq!(fs::read(&f).unwrap(), vec![0u8, 1, 2, 3, 255]);
    assert!(stdout(&o).contains("1 file → dl"), "{}", stdout(&o));
    step.assert_hits(1);

    // Bare product resolves when there is exactly one model.
    let o = sb.sarg_in(&["cad", "get", "heltec-wifi-lora-32-v4", "--file", "amz-heltec-lora-v4.step", "-o", "dl2"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    step.assert_hits(2);
}

#[test]
fn cad_get_signed_out_401_says_sign_in() {
    let sb = Sandbox::new();
    sb.json_get("/m/sarg/heltec-wifi-lora-32-v4", MODEL);
    sb.server.mock(|when, then| {
        when.method(GET).path_contains("/f/");
        then.status(401).header("content-type", "application/json").body(r#"{"error":"sign in to download","hint":"GET /start.md"}"#);
    });
    let o = sb.sarg(&["cad", "get", "sarg/heltec-wifi-lora-32-v4", "--role", "cad"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("downloading geometry needs a token"), "{}", stderr(&o));
}
