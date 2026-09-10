//! The project manifest: init, add, show, new --like, bom check, stock —
//! and the verbs that read the manifest when run inside a project dir.

mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use httpmock::prelude::*;

const API_FIXTURE: &str = include_str!("fixtures/api.json");
const NOTES_FULL: &str = include_str!("fixtures/notes_full.json");
const SEARCH: &str = include_str!("fixtures/search.json");
const PART: &str = include_str!("fixtures/part.json");

fn project_dir(sb: &Sandbox, name: &str) -> PathBuf {
    let d = sb.home.join("code").join(name);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn init_writes_a_manifest_and_registers_the_project() {
    let sb = Sandbox::new();
    let dir = project_dir(&sb, "openmv_n6");
    let o = sb.sarg_in_dir(&["project", "init", "--board", "openmv-n6"], &dir);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("created openmv_n6"), "{out}");
    assert!(
        out.contains("project openmv-n6"),
        "slug from the dir name: {out}"
    );
    let text = fs::read_to_string(dir.join("sarg.yaml")).unwrap();
    assert!(text.contains("name: openmv_n6"), "{text}");
    assert!(text.contains("project: openmv-n6"), "{text}");
    assert!(text.contains("- openmv-n6"), "{text}");
    assert!(!text.contains("like:"), "unset optionals stay out: {text}");

    // registered: projects.<name> → dir, alias <name> → slug
    let cfg = fs::read_to_string(sb.config_path()).unwrap();
    assert!(cfg.contains("openmv_n6"), "{cfg}");
    assert!(cfg.contains("openmv-n6"), "{cfg}");
    let o = sb.sarg_in_dir(&["--json", "project", "ls"], &dir);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v[0]["name"], "openmv_n6");
    assert_eq!(v[0]["present"], true);
    assert_eq!(v[0]["boards"][0], "openmv-n6");

    // a second init refuses without --force
    let o = sb.sarg_in_dir(&["project", "init"], &dir);
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("already exists"), "{}", stderr(&o));
}

#[test]
fn add_grows_boards_parts_bom_and_refs_and_show_reads_them_back() {
    let sb = Sandbox::new();
    let dir = project_dir(&sb, "speedcam");
    sb.sarg_in_dir(&["project", "init"], &dir);
    let o = sb.sarg_in_dir(
        &["project", "add", "openmv-n6", "--board", "--qty", "1"],
        &dir,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("board openmv-n6"), "{}", stdout(&o));
    assert!(
        stdout(&o).contains("bom added openmv-n6 ×1"),
        "{}",
        stdout(&o)
    );
    let o = sb.sarg_in_dir(&["project", "add", "hlk-ld2415h", "--qty", "1"], &dir);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let o = sb.sarg_in_dir(
        &[
            "project",
            "add",
            "--item",
            "M2.5x6 screws",
            "--qty",
            "8",
            "--note",
            "lid",
            "--ref",
            "sargbench2/n6-case",
        ],
        &dir,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // same id again: nothing new, but a qty update is reported
    let o = sb.sarg_in_dir(&["project", "add", "hlk-ld2415h", "--qty", "2"], &dir);
    assert!(
        stdout(&o).contains("bom updated hlk-ld2415h ×2"),
        "{}",
        stdout(&o)
    );
    assert!(!stdout(&o).contains("part hlk-ld2415h"), "{}", stdout(&o));
    // nothing given is a usage error
    let o = sb.sarg_in_dir(&["project", "add"], &dir);
    assert_eq!(code(&o), 2);

    // show, offline, from a subdirectory (discovery walks up)
    let sub = dir.join("firmware/src");
    fs::create_dir_all(&sub).unwrap();
    let o = sb.sarg_in_dir(&["project", "show", "--offline"], &sub);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("speedcam"), "{out}");
    assert!(out.contains("openmv-n6"), "{out}");
    assert!(out.contains("hlk-ld2415h"), "{out}");
    assert!(
        out.contains("×8") && out.contains("M2.5x6 screws") && out.contains("lid"),
        "{out}"
    );
    assert!(out.contains("sargbench2/n6-case"), "{out}");

    let o = sb.sarg_in_dir(&["--json", "project", "show", "--offline"], &sub);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["manifest"]["boards"][0], "openmv-n6");
    assert_eq!(v["manifest"]["parts"][0], "hlk-ld2415h");
    assert_eq!(v["manifest"]["bom"][1]["qty"], 2);
    assert_eq!(v["manifest"]["bom"][2]["item"], "M2.5x6 screws");
    assert_eq!(v["manifest"]["refs"][0], "sargbench2/n6-case");
}

#[test]
fn show_annotates_each_part_with_what_sarg_knows() {
    let sb = Sandbox::new();
    let dir = project_dir(&sb, "mesh");
    sb.sarg_in_dir(
        &["project", "init", "--board", "heltec-wifi-lora-32-v4"],
        &dir,
    );
    let m = sb.json_get("/p/heltec-wifi-lora-32-v4", PART);
    let o = sb.sarg_in_dir(&["project", "show"], &dir);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("Heltec WiFi LoRa 32 V4"), "{out}");
    assert!(out.contains("lessons"), "{out}");
    assert!(
        out.contains("bring-up"),
        "the digest's bring-up is flagged: {out}"
    );
    m.assert_hits(1);
}

#[test]
fn new_like_clones_boards_parts_bom_and_refs_into_a_fresh_dir() {
    let sb = Sandbox::new();
    let src = project_dir(&sb, "openmv_n6");
    sb.sarg_in_dir(
        &[
            "project",
            "init",
            "--board",
            "openmv-n6",
            "--part",
            "dfr0535",
        ],
        &src,
    );
    sb.sarg_in_dir(
        &[
            "project",
            "add",
            "openmv-n6",
            "--qty",
            "1",
            "--ref",
            "sargbench2/n6-case",
        ],
        &src,
    );

    // by directory
    let dst = sb.home.join("code/n6_v2");
    let o = sb.sarg_in_dir(
        &[
            "project",
            "new",
            dst.to_str().unwrap(),
            "--like",
            src.to_str().unwrap(),
        ],
        &sb.home,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("created n6_v2 like openmv_n6"), "{out}");
    assert!(
        out.contains("carried over: 1 board, 1 part, 1 bom line, 1 ref"),
        "{out}"
    );
    assert!(out.contains("sarg bom check"), "{out}");
    let text = fs::read_to_string(dst.join("sarg.yaml")).unwrap();
    assert!(text.contains("name: n6_v2"), "{text}");
    assert!(text.contains("project: n6-v2"), "{text}");
    assert!(text.contains("like: openmv-n6"), "{text}");
    assert!(text.contains("- dfr0535"), "{text}");
    assert!(text.contains("sargbench2/n6-case"), "{text}");

    // by registered name, with an extra board, custom slug
    let dst2 = sb.home.join("code/n6_solar");
    let o = sb.sarg_in_dir(
        &[
            "--json",
            "project",
            "new",
            dst2.to_str().unwrap(),
            "--like",
            "openmv_n6",
            "--board",
            "dfr0535",
            "--project",
            "n6-solar-node",
        ],
        &sb.home,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["like"], "openmv-n6");
    assert_eq!(v["manifest"]["project"], "n6-solar-node");
    assert_eq!(v["manifest"]["boards"][1], "dfr0535");

    // by slug of a registered project
    let dst3 = sb.home.join("code/n6_three");
    let o = sb.sarg_in_dir(
        &[
            "project",
            "new",
            dst3.to_str().unwrap(),
            "--like",
            "n6-solar-node",
        ],
        &sb.home,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));

    // unknown source names what is registered
    let o = sb.sarg_in_dir(&["project", "new", "x", "--like", "nope"], &sb.home);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("openmv_n6"), "{}", stderr(&o));
    assert!(
        !sb.home.join("x").exists(),
        "nothing created on a bad --like"
    );

    // ls shows the lineage
    let o = sb.sarg_in_dir(&["project", "ls"], &sb.home);
    let out = stdout(&o);
    assert!(
        out.contains("n6_v2") && out.contains("like openmv-n6"),
        "{out}"
    );
}

#[test]
fn bom_check_uses_stock_then_sargineer_and_lists_what_to_buy() {
    let sb = Sandbox::new();
    let dir = project_dir(&sb, "speedcam");
    sb.sarg_in_dir(&["project", "init"], &dir);
    sb.sarg_in_dir(
        &[
            "project",
            "add",
            "heltec-wifi-lora-32-v4",
            "--board",
            "--qty",
            "1",
        ],
        &dir,
    );
    sb.sarg_in_dir(&["project", "add", "hlk-ld2415h", "--qty", "2"], &dir);
    sb.sarg_in_dir(
        &["project", "add", "--item", "M2.5x6 screws", "--qty", "8"],
        &dir,
    );
    sb.sarg_in_dir(
        &["project", "add", "--item", "18650 cell", "--qty", "1"],
        &dir,
    );

    // heltec: the fixture says qty_owned 2 → on hand. radar: not owned, buy link.
    sb.json_get("/p/heltec-wifi-lora-32-v4", PART);
    sb.json_get(
        "/p/hlk-ld2415h",
        r#"{"part":{"id":"hlk-ld2415h","name":"HLK-LD2415H radar","buy":["https://example.com/ld2415h"],"owned_qty":0},"facts":{}}"#,
    );
    // stock file knows the screws; the cell is untracked
    let o = sb.sarg_in_dir(
        &["stock", "set", "M2.5x6 screws", "20", "--where", "bin A3"],
        &dir,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(
        stdout(&o).contains("m2-5x6-screws ×20 @ bin A3"),
        "{}",
        stdout(&o)
    );

    let o = sb.sarg_in_dir(&["bom", "check"], &dir);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("✓ heltec-wifi-lora-32-v4"), "{out}");
    assert!(out.contains("have 2"), "{out}");
    assert!(
        out.contains("✗ hlk-ld2415h")
            && out.contains("short 2")
            && out.contains("https://example.com/ld2415h"),
        "{out}"
    );
    assert!(
        out.contains("✓ M2.5x6 screws") && out.contains("have 20"),
        "{out}"
    );
    assert!(
        out.contains("? 18650 cell") && out.contains("sarg stock set 18650-cell"),
        "{out}"
    );
    assert!(
        out.contains("on hand 2 of 4")
            && out.contains("buy 1: hlk-ld2415h")
            && out.contains("untracked 1"),
        "{out}"
    );

    let o = sb.sarg_in_dir(&["--json", "bom", "check"], &dir);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["on_hand"], 2);
    assert_eq!(v["buy"], 1);
    assert_eq!(v["untracked"], 1);
    assert_eq!(v["lines"][0]["source"], "sargineer");
    assert_eq!(v["lines"][2]["source"], "stock");
    assert_eq!(v["lines"][1]["buy"], "https://example.com/ld2415h");

    // stock overrides the server; offline uses only the stock file
    sb.sarg_in_dir(&["stock", "set", "hlk-ld2415h", "3"], &dir);
    let o = sb.sarg_in_dir(&["--json", "bom", "check", "--offline"], &dir);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["lines"][1]["state"], "on-hand");
    assert_eq!(
        v["lines"][0]["state"], "untracked",
        "offline: no server answer for the heltec"
    );

    let o = sb.sarg_in_dir(&["stock", "ls"], &dir);
    assert!(
        stdout(&o).contains("hlk-ld2415h") && stdout(&o).contains("bin A3"),
        "{}",
        stdout(&o)
    );
    let o = sb.sarg_in_dir(&["stock", "rm", "hlk-ld2415h"], &dir);
    assert!(stdout(&o).contains("removed hlk-ld2415h"), "{}", stdout(&o));
    let stock = fs::read_to_string(sb.home.join(".sargineer/stock.yaml")).unwrap();
    assert!(!stock.contains("hlk-ld2415h"), "{stock}");
    assert!(stock.contains("m2-5x6-screws"), "{stock}");
}

#[test]
fn preflight_inside_a_project_takes_its_boards() {
    let sb = Sandbox::new();
    let dir = project_dir(&sb, "mesh");
    sb.sarg_in_dir(&["project", "init", "--board", "heltec v4"], &dir);
    sb.server.mock(|when, then| {
        when.method(GET)
            .path("/search")
            .query_param("q", "heltec v4");
        then.status(200)
            .header("content-type", "application/json")
            .body(SEARCH);
    });
    let notes = sb.server.mock(|when, then| {
        when.method(GET)
            .path("/notes")
            .query_param("q", "heltec v4");
        then.status(200)
            .header("content-type", "application/json")
            .body(NOTES_FULL);
    });
    let o = sb.sarg_in_dir(&["preflight"], &dir);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("project mesh"), "{out}");
    assert!(out.contains("board heltec v4"), "{out}");
    notes.assert_hits(1);

    // outside any project, no boards is a usage error that says what to do
    let o = sb.sarg_in(&["preflight"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("sarg.yaml"), "{}", stderr(&o));
}

#[test]
fn ask_inside_a_project_names_it_and_ranks_its_boards_first() {
    let sb = Sandbox::new();
    let dir = project_dir(&sb, "mesh");
    sb.sarg_in_dir(
        &["project", "init", "--board", "heltec-wifi-lora-32-v4"],
        &dir,
    );
    sb.server.mock(|when, then| {
        when.method(GET).path("/search");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"q":"usb","parts":[],"notes":[]}"#);
    });
    sb.server.mock(|when, then| {
        when.method(GET).path("/notes");
        then.status(200).header("content-type", "application/json").body(
            r#"{"count":2,"total":2,"results":[
              {"handle":"a","id":"other","title":"USB on a pico","status":"working","hw":["rp2040"],"fix":"x"},
              {"handle":"b","id":"mine","title":"USB on the heltec","status":"working","hw":["heltec-wifi-lora-32-v4"],"fix":"y"}
            ]}"#,
        );
    });
    let o = sb.sarg_in_dir(&["ask", "usb"], &dir);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains("project mesh · boards heltec-wifi-lora-32-v4 rank first"),
        "{out}"
    );
    let first = out
        .find("b/mine")
        .expect("the project's board hit is present");
    let second = out.find("a/other").expect("the other hit is present");
    assert!(first < second, "project board ranks first:\n{out}");

    let o = sb.sarg_in_dir(&["--json", "ask", "usb"], &dir);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["project"]["project"], "mesh");
    assert_eq!(v["notes"][0]["id"], "mine");
}

#[test]
fn lesson_new_inside_a_project_fills_project_and_hw() {
    let sb = Sandbox::new();
    sb.json_get("/api", API_FIXTURE);
    let dir = project_dir(&sb, "speedcam");
    sb.sarg_in_dir(
        &[
            "project",
            "init",
            "--project",
            "n6-speedcam",
            "--board",
            "openmv-n6",
        ],
        &dir,
    );
    let post = sb.server.mock(|when, then| {
        when.method(POST)
            .path("/notes")
            .body_contains("\"project\":\"n6-speedcam\"")
            .body_contains("\"hw\":[\"openmv-n6\"]");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"x","handle":"me"}"#);
    });
    let o = sb.sarg_in_dir(
        &[
            "lesson",
            "new",
            "--title",
            "OpenMV N6 radar UART drops bytes at 256000 baud unless rx buffer 0x400",
            "--symptom",
            "frames truncated",
            "--fix",
            "set the buffer",
            "--intent",
            "read the radar",
        ],
        &dir,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    post.assert_hits(1);

    // explicit flags still win over the manifest
    let post2 = sb.server.mock(|when, then| {
        when.method(POST)
            .path("/notes")
            .body_contains("\"project\":\"bench\"")
            .body_contains("\"hw\":[\"esp32-s3\"]");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"id":"y","handle":"me"}"#);
    });
    let o = sb.sarg_in_dir(
        &[
            "lesson",
            "new",
            "--title",
            "flash write fails at 0x0 on esp32-s3 error 0x05",
            "--symptom",
            "stopped",
            "--fix",
            "erase first",
            "--intent",
            "flash",
            "--project",
            "bench",
            "--hw",
            "esp32-s3",
        ],
        &dir,
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    post2.assert_hits(1);
}

#[test]
fn add_never_lists_a_board_as_a_part_too_and_board_promotes() {
    let sb = Sandbox::new();
    let dir = project_dir(&sb, "p");
    sb.sarg_in_dir(&["project", "init", "--board", "openmv-n6"], &dir);
    sb.sarg_in_dir(&["project", "add", "openmv-n6", "--qty", "1"], &dir);
    sb.sarg_in_dir(&["project", "add", "dfr0535"], &dir);
    sb.sarg_in_dir(&["project", "add", "dfr0535", "--board"], &dir);
    let o = sb.sarg_in_dir(&["--json", "project", "show", "--offline"], &dir);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(
        v["manifest"]["boards"],
        serde_json::json!(["openmv-n6", "dfr0535"])
    );
    assert_eq!(v["manifest"]["parts"], serde_json::json!([]));
    assert_eq!(v["manifest"]["bom"][0]["product"], "openmv-n6");
}
