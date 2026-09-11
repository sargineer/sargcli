//! Telling sarg: compose, validate, scan and post notes; publish (the
//! user's own act); and upload what was spooled while offline. Everything
//! written lands private.

use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use super::Ctx;
use crate::api;
use crate::cli::{LessonCmd, NoteArgs};
use crate::client::urlencode;
use crate::error::{Result, SargError};
use crate::manifest;
use crate::notes;
use crate::output;
use crate::render::{self, s};
use crate::secrets;

pub fn lesson(ctx: &mut Ctx, cmd: &LessonCmd) -> Result<()> {
    match cmd {
        LessonCmd::New(args) => new(ctx, args, "lesson"),
        LessonCmd::Ls { hw, n } => ls(ctx, hw.as_deref(), *n),
        LessonCmd::Edit { r#ref, allow_pii } => edit(ctx, r#ref, *allow_pii),
        LessonCmd::Rm { r#ref } => rm(ctx, r#ref),
        LessonCmd::Publish {
            r#ref,
            user_authorized,
        } => set_visibility(ctx, r#ref, true, *user_authorized),
        LessonCmd::Hide {
            r#ref,
            user_authorized,
        } => set_visibility(ctx, r#ref, false, *user_authorized),
    }
}

pub fn issue(ctx: &mut Ctx, args: &NoteArgs) -> Result<()> {
    new(ctx, args, "issue")
}
pub fn feedback(ctx: &mut Ctx, args: &NoteArgs) -> Result<()> {
    new(ctx, args, "feedback")
}

/// Build a note payload from -f / --edit / flags, in that order of preference.
fn assemble(ctx: &Ctx, args: &NoteArgs, kind: &str, note_fields: &Value) -> Result<Value> {
    let mut note: Value = if let Some(f) = &args.file {
        let text = if f == "@-" {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        } else {
            fs::read_to_string(f).map_err(|e| SargError::usage(format!("cannot read {f}: {e}")))?
        };
        notes::parse_note(&text)?
    } else {
        Value::Object(Map::new())
    };

    // Overlay explicit flags.
    let obj = note.as_object_mut().ok_or_else(|| SargError::Validation {
        message: "the note must be an object".into(),
        hint: None,
    })?;
    let mut set_str = |k: &str, v: &Option<String>| {
        if let Some(val) = v {
            obj.insert(k.into(), json!(val));
        }
    };
    set_str("title", &args.title);
    set_str("symptom", &args.symptom);
    set_str("cause", &args.cause);
    set_str("fix", &args.fix);
    set_str("setup", &args.setup);
    set_str("check", &args.check);
    set_str("intent", &args.intent);
    set_str("about", &args.about);
    set_str("body", &args.body);
    if let Some(st) = &args.status {
        obj.insert("status".into(), json!(st));
    }
    let split = |xs: &[String]| -> Vec<String> {
        xs.iter()
            .flat_map(|x| x.split(',').map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect()
    };
    if !args.hw.is_empty() {
        obj.insert("hw".into(), json!(split(&args.hw)));
    }
    if !args.sw.is_empty() {
        obj.insert("sw".into(), json!(split(&args.sw)));
    }
    if !args.steps.is_empty() {
        obj.insert("steps".into(), json!(args.steps));
    }
    if !args.unverified.is_empty() {
        obj.insert("unverified".into(), json!(args.unverified));
    }

    // Defaults the server needs but that are cheap to supply.
    obj.entry("kind").or_insert(json!(kind));
    obj.entry("status").or_insert(json!("working"));
    // project (required for lessons): flag → alias → the manifest here → default_project.
    // The manifest's boards also fill `hw` when nothing named the hardware.
    let found = manifest::discover();
    if kind == "lesson"
        && obj
            .get("project")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .is_empty()
    {
        if let Some(p) = args
            .project
            .clone()
            .map(|p| ctx.cfg.project_aliases.get(&p).cloned().unwrap_or(p))
            .or_else(|| found.as_ref().map(|f| f.manifest.project.clone()))
            .or_else(|| ctx.cfg.default_project.clone())
        {
            obj.insert("project".into(), json!(p));
        }
    } else if let Some(p) = &args.project {
        let resolved = ctx
            .cfg
            .project_aliases
            .get(p)
            .cloned()
            .unwrap_or_else(|| p.clone());
        obj.insert("project".into(), json!(resolved));
    }
    if kind == "lesson"
        && obj
            .get("hw")
            .map(|h| h.as_array().is_none_or(|a| a.is_empty()))
            .unwrap_or(true)
    {
        if let Some(f) = &found {
            if !f.manifest.boards.is_empty() {
                obj.insert("hw".into(), json!(f.manifest.boards));
            }
        }
    }
    // host tags from config, unless the note set its own.
    if !ctx.cfg.host_tags.is_empty()
        && obj
            .get("host")
            .map(|h| h.as_array().is_none_or(|a| a.is_empty()))
            .unwrap_or(true)
    {
        obj.insert("host".into(), json!(ctx.cfg.host_tags));
    }

    // If --edit, hand the assembled note to the editor as a template.
    if args.edit {
        let seed = if note.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            note_fields.get("example").cloned().unwrap_or(note.clone())
        } else {
            note.clone()
        };
        let mut seed = seed;
        if let Some(o) = seed.as_object_mut() {
            o.insert("kind".into(), json!(kind));
        }
        note = open_editor(&notes::edit_template(&seed, kind))?;
    }

    // Author is taken from the token; never send it.
    if let Some(o) = note.as_object_mut() {
        o.remove("author");
        o.remove("handle");
        o.remove("visibility");
    }
    Ok(note)
}

fn open_editor(template: &str) -> Result<Value> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let path = std::env::temp_dir().join(format!("sarg-note-{}.toml", std::process::id()));
    fs::write(&path, template)?;
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| SargError::usage(format!("cannot launch $EDITOR ({editor}): {e}")))?;
    if !status.success() {
        let _ = fs::remove_file(&path);
        return Err(SargError::usage("editor exited without saving"));
    }
    let text = fs::read_to_string(&path)?;
    let _ = fs::remove_file(&path);
    notes::parse_note(&text)
}

/// Scan a note about to leave the machine; refuse or, when `allow_pii` lets
/// soft findings through, say what is being sent.
fn gate(ctx: &Ctx, note: &Value, allow_pii: bool) -> Result<()> {
    let findings = secrets::scan(note, &ctx.cfg, ctx.resolved.token.as_deref());
    if let Some((message, hint)) = secrets::refusal(&findings, allow_pii) {
        return Err(SargError::Validation {
            message,
            hint: Some(hint.into()),
        });
    }
    for f in findings.iter().filter(|f| !f.hard()) {
        eprintln!(
            "{}",
            render::warn(&format!(
                "sarg: sending {} in `{}` ({})",
                f.kind, f.field, f.sample
            ))
        );
    }
    Ok(())
}

fn new(ctx: &mut Ctx, args: &NoteArgs, kind: &str) -> Result<()> {
    let paths = ctx.paths.clone();
    let env = ctx.resolved.env.clone();
    let api_info = api::load(&ctx.client, &paths, &mut ctx.cfg, &env, ctx.cli.no_cache)?;
    let note = assemble(ctx, args, kind, &api_info.note_fields)?;

    // 1. validate against the live rules.
    let spec = notes::spec_for_kind(&api_info.note_fields, kind);
    let warnings = notes::validate(&note, &spec)?;

    // 2. nothing sensitive leaves the machine.
    gate(ctx, &note, args.allow_pii)?;
    for w in &warnings {
        eprintln!("{}", render::warn(&format!("sarg: {w}")));
    }

    if args.dry_run {
        if ctx.json() {
            output::json(&json!({"ok": true, "would_send": note, "warnings": warnings}));
        } else {
            eprintln!("{}", render::good("dry run: valid and clean, not sent"));
            println!("{}", serde_json::to_string_pretty(&note)?);
        }
        return Ok(());
    }

    // 3. send, or spool if we cannot.
    if ctx.resolved.token.is_none() {
        return spool(ctx, &note, "no token");
    }
    match ctx.client.post_json("/notes", &note) {
        Ok(r) => {
            let v = r.json().unwrap_or(Value::Null);
            report_created(ctx, kind, &v);
            Ok(())
        }
        Err(SargError::Offline(_)) => spool(ctx, &note, "offline"),
        Err(e) => Err(e),
    }
}

fn report_created(ctx: &Ctx, kind: &str, v: &Value) {
    let id = s(v, "id");
    let handle = if s(v, "handle").is_empty() {
        "you"
    } else {
        s(v, "handle")
    };
    if ctx.json() {
        output::json(v);
        return;
    }
    println!(
        "{} {} saved private · {}/{}",
        render::good("✓"),
        kind,
        handle,
        id
    );
    if kind == "lesson" {
        println!(
            "{}",
            render::dim(&format!(
                "publish is your call: sarg lesson publish {handle}/{id}"
            ))
        );
    }
}

fn spool(ctx: &Ctx, note: &Value, why: &str) -> Result<()> {
    let dir = ctx.paths.pending_dir.clone();
    fs::create_dir_all(&dir)?;
    let slug: String = s(note, "title")
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(48)
        .collect();
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let name = format!("{ts}-{}.json", if slug.is_empty() { "note" } else { &slug });
    let path = dir.join(&name);
    fs::write(&path, serde_json::to_vec_pretty(note)?)?;
    if ctx.json() {
        output::json(&json!({"spooled": path, "reason": why}));
    } else {
        eprintln!(
            "{}",
            render::warn(&format!(
                "{why}: spooled to {} — sarg sync when connected",
                path.display()
            ))
        );
    }
    Err(SargError::Offline(format!("spooled ({why})")))
}

pub fn sync(ctx: &mut Ctx) -> Result<()> {
    let dir = ctx.paths.pending_dir.clone();
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    files.sort();
    if files.is_empty() {
        println!("nothing spooled");
        return Ok(());
    }
    if ctx.resolved.token.is_none() {
        return Err(SargError::Auth {
            message: format!("{} spooled note(s) but no token", files.len()),
            hint: Some("sarg config set token <token>".into()),
        });
    }
    let mut sent = 0;
    let mut failed = 0;
    for f in &files {
        let text = fs::read_to_string(f)?;
        let note: Value = serde_json::from_str(&text)?;
        match ctx.client.post_json("/notes", &note) {
            Ok(r) => {
                let v = r.json().unwrap_or(Value::Null);
                fs::remove_file(f)?;
                sent += 1;
                println!(
                    "{} {} → {}/{}",
                    render::good("✓"),
                    f.file_name().unwrap().to_string_lossy(),
                    s(&v, "handle"),
                    s(&v, "id")
                );
            }
            Err(SargError::Offline(_)) => {
                return Err(SargError::Offline(format!(
                    "offline — {sent} sent, {} still spooled",
                    files.len() - sent
                )))
            }
            Err(e) => {
                failed += 1;
                eprintln!("{} {}: {e}", render::warn("kept"), f.display());
            }
        }
    }
    println!("{sent} sent, {failed} kept");
    Ok(())
}

fn ls(ctx: &mut Ctx, hw: Option<&str>, n: usize) -> Result<()> {
    let mut path = format!("/notes?mine=1&n={n}");
    if let Some(h) = hw {
        path.push_str(&format!("&hw={}", urlencode(h)));
    }
    let v = ctx.client.get_json(&path)?;
    if ctx.json() {
        output::json(&v);
        return Ok(());
    }
    let results = v
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    println!(
        "your notes: {} of {}",
        results.len(),
        v.get("total").and_then(|t| t.as_u64()).unwrap_or(0)
    );
    let w = render::width();
    for nt in &results {
        println!(
            "  {} {} {}",
            render::status_badge(s(nt, "status")),
            render::bold(&render::note_ref(nt)),
            render::dim(&render::truncate(s(nt, "title"), w.saturating_sub(30)))
        );
    }
    Ok(())
}

fn split_ref(r: &str) -> (Option<&str>, &str) {
    let r = r.trim().trim_start_matches("/n/").trim_start_matches('/');
    match r.split_once('/') {
        Some((h, id)) => (Some(h), id),
        None => (None, r),
    }
}

fn edit(ctx: &mut Ctx, r#ref: &str, allow_pii: bool) -> Result<()> {
    let (handle, id) = split_ref(r#ref);
    let handle = handle.ok_or_else(|| SargError::usage("give <handle>/<id> to edit"))?;
    let current = ctx.client.get_json(&format!("/n/{handle}/{id}"))?;
    let kind = s(&current, "kind");
    let kind = if kind.is_empty() { "lesson" } else { kind };
    let mut seed = current.clone();
    if let Some(o) = seed.as_object_mut() {
        for k in ["author", "handle", "visibility", "date", "updated"] {
            o.remove(k);
        }
    }
    let edited = open_editor(&notes::edit_template(&seed, kind))?;
    let paths = ctx.paths.clone();
    let env = ctx.resolved.env.clone();
    let api_info = api::load(&ctx.client, &paths, &mut ctx.cfg, &env, false)?;
    let spec = notes::spec_for_kind(&api_info.note_fields, kind);
    notes::validate(&edited, &spec)?;
    gate(ctx, &edited, allow_pii)?;
    let r = ctx.client.ok(ctx.client.send(
        "PUT",
        &format!("/n/{handle}/{id}"),
        Some(&edited.to_string()),
        &[("Content-Type".into(), "application/json".into())],
    )?)?;
    let _ = r;
    println!(
        "{} updated {handle}/{id} (id and publish state kept)",
        render::good("✓")
    );
    Ok(())
}

fn rm(ctx: &mut Ctx, r#ref: &str) -> Result<()> {
    let (handle, id) = split_ref(r#ref);
    let handle = handle.ok_or_else(|| SargError::usage("give <handle>/<id> to remove"))?;
    if std::io::stdin().is_terminal() {
        print!("delete {handle}/{id}? [y/N] ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !line.trim().eq_ignore_ascii_case("y") {
            return Err(SargError::usage("cancelled"));
        }
    }
    ctx.client.ok(ctx
        .client
        .send("DELETE", &format!("/n/{handle}/{id}"), None, &[])?)?;
    println!("{} deleted {handle}/{id}", render::good("✓"));
    Ok(())
}

/// Publish and hide are the user's own act. An agent (no TTY) must pass
/// --user-authorized; a person confirms at the prompt.
fn set_visibility(ctx: &mut Ctx, r#ref: &str, publish: bool, authorized: bool) -> Result<()> {
    let (handle, id) = split_ref(r#ref);
    let handle = handle.ok_or_else(|| SargError::usage("give <handle>/<id>"))?;
    let verb = if publish { "publish" } else { "hide" };
    let tty = std::io::stdin().is_terminal();
    if !authorized {
        if tty {
            print!(
                "{} {handle}/{id} — this makes it {}. Confirm? [y/N] ",
                verb,
                if publish { "public" } else { "private" }
            );
            std::io::stdout().flush().ok();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if !line.trim().eq_ignore_ascii_case("y") {
                return Err(SargError::usage("cancelled"));
            }
        } else {
            return Err(SargError::usage(format!(
                "{verb} is the user's own act — re-run with --user-authorized once they have said so"
            )));
        }
    }
    // The server answers with an empty 303; follow up with a GET to report state.
    ctx.client
        .ok(ctx.client.post_empty(&format!("/n/{handle}/{id}/{verb}"))?)?;
    let after = ctx.client.get_json(&format!("/n/{handle}/{id}"))?;
    let vis = s(&after, "visibility");
    if ctx.json() {
        output::json(&json!({"ref": format!("{handle}/{id}"), "visibility": vis}));
    } else {
        println!(
            "{} {handle}/{id} is now {}",
            render::good("✓"),
            if vis.is_empty() { verb } else { vis }
        );
        if publish && vis == "public" {
            println!(
                "{}",
                render::dim(&format!("{}/n/{handle}/{id}", ctx.resolved.url))
            );
        }
    }
    Ok(())
}
