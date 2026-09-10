//! `sarg project`: the manifest verbs. `init` starts one here, `add` grows
//! it, `show` reads it back with what sarg knows about each part, `ls`
//! lists the ones this machine has registered, and `new --like` clones a
//! project that already works — boards, parts, BOM, refs — into a fresh
//! directory. That clone is the fast path: start from known-good.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::Ctx;
use crate::cli::{ProjectAddArgs, ProjectCmd, ProjectInitArgs, ProjectNewArgs};
use crate::error::{Result, SargError};
use crate::manifest::{self, Found, Manifest};
use crate::output;
use crate::render::{self, s, u};

pub fn run(ctx: &mut Ctx, cmd: &ProjectCmd) -> Result<()> {
    match cmd {
        ProjectCmd::Init(args) => init(ctx, args),
        ProjectCmd::Show { dir, offline } => show(ctx, dir.as_deref(), *offline),
        ProjectCmd::Add(args) => add(ctx, args),
        ProjectCmd::New(args) => new(ctx, args),
        ProjectCmd::Ls => ls(ctx),
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".into())
}

/// Remember the project in config: `projects.<name>` → dir, and the alias
/// `<name>` → slug so `lesson new --project <name>` resolves anywhere.
fn register(ctx: &mut Ctx, found: &Found) -> Result<()> {
    let name = found.manifest.name.clone();
    ctx.cfg
        .projects
        .insert(name.clone(), found.dir.display().to_string());
    ctx.cfg
        .project_aliases
        .entry(name)
        .or_insert_with(|| found.manifest.project.clone());
    ctx.cfg.save(&ctx.paths)
}

fn init(ctx: &mut Ctx, args: &ProjectInitArgs) -> Result<()> {
    let dir = match &args.dir {
        Some(d) => {
            fs::create_dir_all(d)?;
            fs::canonicalize(d)?
        }
        None => std::env::current_dir()?,
    };
    if dir.join(manifest::FILE).exists() && !args.force {
        return Err(SargError::usage(format!(
            "{} already exists — sarg project show, or --force to overwrite",
            dir.join(manifest::FILE).display()
        )));
    }
    let name = args.name.clone().unwrap_or_else(|| dir_name(&dir));
    let project = args
        .project
        .clone()
        .unwrap_or_else(|| manifest::slugify(&name));
    let mut m = Manifest {
        name,
        project,
        created: Some(today()),
        ..Default::default()
    };
    for b in &args.boards {
        Manifest::push_unique(&mut m.boards, b);
    }
    for p in &args.parts {
        Manifest::push_unique(&mut m.parts, p);
    }
    let found = Found { dir, manifest: m };
    found.save()?;
    register(ctx, &found)?;
    report_written(ctx, &found, "created");
    Ok(())
}

fn report_written(ctx: &Ctx, found: &Found, what: &str) {
    if ctx.json() {
        output::json(&json!({
            "ok": true, "action": what, "dir": found.dir, "file": found.path(),
            "manifest": found.manifest,
        }));
        return;
    }
    println!(
        "{} {} {} · project {} · {}",
        render::good("✓"),
        what,
        render::bold(&found.manifest.name),
        found.manifest.project,
        render::dim(&found.path().display().to_string())
    );
    let m = &found.manifest;
    if !m.boards.is_empty() {
        println!("  boards  {}", m.boards.join(" "));
    }
    if !m.parts.is_empty() {
        println!("  parts   {}", m.parts.join(" "));
    }
    if !m.bom.is_empty() {
        println!(
            "  bom     {} line{}",
            m.bom.len(),
            if m.bom.len() == 1 { "" } else { "s" }
        );
    }
    if !m.refs.is_empty() {
        println!("  refs    {}", m.refs.join(" "));
    }
    println!(
        "{}",
        render::dim("next: sarg project add <product-id> [--board] [--qty N] · sarg preflight · sarg bom check")
    );
}

fn add(ctx: &mut Ctx, args: &ProjectAddArgs) -> Result<()> {
    let mut found = manifest::find(args.dir.as_deref())?;
    let mut changes: Vec<String> = Vec::new();
    for id in &args.ids {
        let id = id.trim().trim_start_matches("/p/");
        let m = &mut found.manifest;
        if args.board {
            // promoting a part to a board moves it
            m.parts.retain(|p| p != id);
        }
        let already_board = m.boards.iter().any(|b| b == id);
        let list = if args.board {
            &mut m.boards
        } else {
            &mut m.parts
        };
        if !(already_board && !args.board) && Manifest::push_unique(list, id) {
            changes.push(format!(
                "{} {id}",
                if args.board { "board" } else { "part" }
            ));
        }
        if let Some(q) = args.qty {
            let r = found
                .manifest
                .set_bom(Some(id), None, q, args.note.as_deref());
            changes.push(format!("bom {r} {id} ×{q}"));
        }
    }
    if let Some(item) = &args.item {
        let q = args.qty.unwrap_or(1);
        let r = found
            .manifest
            .set_bom(None, Some(item), q, args.note.as_deref());
        changes.push(format!("bom {r} {item} ×{q}"));
    }
    for r in &args.refs {
        if Manifest::push_unique(&mut found.manifest.refs, r) {
            changes.push(format!("ref {r}"));
        }
    }
    if args.ids.is_empty() && args.item.is_none() && args.refs.is_empty() {
        return Err(SargError::usage(
            "nothing to add: give product ids, --item <text> [--qty N], or --ref <handle>/<id>",
        ));
    }
    found.save()?;
    if ctx.json() {
        output::json(&json!({"ok": true, "changes": changes, "manifest": found.manifest}));
        return Ok(());
    }
    if changes.is_empty() {
        println!("{}", render::dim("nothing new — already in the manifest"));
    }
    for c in &changes {
        println!("{} {c}", render::good("+"));
    }
    Ok(())
}

/// What sarg knows about one product, in one line. Errors become text so
/// an unreachable server never hides the manifest itself.
fn live_line(ctx: &Ctx, id: &str) -> Value {
    match ctx.client.get_json(&format!("/p/{id}")) {
        Ok(v) => {
            let part = v.get("part").cloned().unwrap_or(Value::Null);
            let facts = v.get("facts").cloned().unwrap_or(Value::Null);
            let lessons = v
                .get("related")
                .and_then(|r| r.get("notes"))
                .and_then(|n| n.get("exact"))
                .and_then(|e| e.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let models = v
                .get("models")
                .and_then(|m| m.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let owned = u(&part, "owned_qty").max(u(&facts, "qty_owned"));
            let name = if s(&part, "name").is_empty() {
                s(&facts, "name")
            } else {
                s(&part, "name")
            };
            let bringup = v
                .get("digest")
                .and_then(|d| d.get("bringup"))
                .and_then(|b| b.get("source"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            json!({
                "product": id, "name": name, "lessons": lessons, "models": models,
                "owned": owned, "bringup": bringup, "ok": true,
            })
        }
        Err(e) => json!({"product": id, "ok": false, "error": e.to_string()}),
    }
}

fn render_live(l: &Value) -> String {
    if l.get("ok").and_then(|b| b.as_bool()) != Some(true) {
        return render::dim(&format!("({})", render::truncate(s(l, "error"), 50)));
    }
    let mut bits = Vec::new();
    if !s(l, "name").is_empty() {
        bits.push(render::truncate(s(l, "name"), 44));
    }
    bits.push(render::dim(&format!("{} lessons", u(l, "lessons"))));
    let models = u(l, "models");
    if models > 0 {
        bits.push(render::dim(&format!(
            "{models} model{}",
            if models == 1 { "" } else { "s" }
        )));
    }
    if u(l, "owned") > 0 {
        bits.push(render::good(&format!("owned ×{}", u(l, "owned"))));
    }
    if !s(l, "bringup").is_empty() {
        bits.push(render::good("bring-up"));
    }
    bits.join(" · ")
}

fn show(ctx: &mut Ctx, dir: Option<&Path>, offline: bool) -> Result<()> {
    let found = manifest::find(dir)?;
    let m = &found.manifest;
    let products = found.products();
    let live: Vec<Value> = if offline {
        Vec::new()
    } else {
        products.iter().map(|id| live_line(ctx, id)).collect()
    };
    if ctx.json() {
        output::json(&json!({
            "dir": found.dir, "file": found.path(), "manifest": m, "live": live,
        }));
        return Ok(());
    }
    let mut head = vec![render::bold(&m.name), format!("project {}", m.project)];
    if let Some(l) = &m.like {
        head.push(format!("like {l}"));
    }
    if let Some(c) = &m.created {
        head.push(render::dim(c));
    }
    println!("{}", head.join(" · "));
    println!("{}", render::dim(&found.path().display().to_string()));
    let find = |id: &str| live.iter().find(|l| s(l, "product") == id);
    let section = |label: &str, ids: &[String]| {
        if ids.is_empty() {
            return;
        }
        println!();
        println!("{}", render::dim(label));
        for id in ids {
            match find(id) {
                Some(l) => println!("  {:<28} {}", render::bold(id), render_live(l)),
                None => println!("  {}", render::bold(id)),
            }
        }
    };
    section("boards", &m.boards);
    section("parts", &m.parts);
    if !m.bom.is_empty() {
        println!();
        println!("{}", render::dim("bom"));
        for line in &m.bom {
            let note = line
                .note
                .as_deref()
                .map(|n| render::dim(&format!("  {n}")))
                .unwrap_or_default();
            println!("  ×{:<4} {}{}", line.qty, line.label(), note);
        }
    }
    if !m.refs.is_empty() {
        println!();
        println!("{}", render::dim("refs"));
        for r in &m.refs {
            println!("  {r}");
        }
    }
    if let Some(n) = &m.notes {
        println!();
        println!("{}", render::wrap(n, render::width(), 2));
    }
    if !m.deny_terms.is_empty() {
        println!();
        println!(
            "{}",
            render::dim(&format!(
                "deny_terms (project): {}",
                m.deny_terms.join(", ")
            ))
        );
    }
    if offline && !products.is_empty() {
        println!();
        println!(
            "{}",
            render::dim("offline: drop --offline to see lessons, models and ownership per part")
        );
    }
    Ok(())
}

/// Resolve `--like`: a directory holding sarg.yaml, or a registered name.
fn resolve_like(ctx: &Ctx, like: &str) -> Result<Found> {
    let p = Path::new(like);
    if p.join(manifest::FILE).is_file() {
        return manifest::find(Some(p));
    }
    if let Some(dir) = ctx.cfg.projects.get(like) {
        let d = PathBuf::from(dir);
        if d.join(manifest::FILE).is_file() {
            return manifest::find(Some(&d));
        }
        return Err(SargError::usage(format!(
            "project {like} is registered at {dir} but has no {} any more",
            manifest::FILE
        )));
    }
    // A slug match on a registered project.
    for (name, dir) in &ctx.cfg.projects {
        if let Ok(f) = manifest::find(Some(Path::new(dir))) {
            if f.manifest.project == like || name == like {
                return Ok(f);
            }
        }
    }
    let known: Vec<&String> = ctx.cfg.projects.keys().collect();
    Err(SargError::usage(format!(
        "no project \"{like}\" — give a directory with {}, or one of: {}",
        manifest::FILE,
        if known.is_empty() {
            "(none registered; sarg project init registers one)".to_string()
        } else {
            known
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    )))
}

fn new(ctx: &mut Ctx, args: &ProjectNewArgs) -> Result<()> {
    let dir = &args.dir;
    if dir.join(manifest::FILE).exists() {
        return Err(SargError::usage(format!(
            "{} already has a {} — sarg project show {}",
            dir.display(),
            manifest::FILE,
            dir.display()
        )));
    }
    let source = match &args.like {
        Some(l) => Some(resolve_like(ctx, l)?),
        None => None,
    };
    fs::create_dir_all(dir)?;
    let dir = fs::canonicalize(dir)?;
    let name = args.name.clone().unwrap_or_else(|| dir_name(&dir));
    let project = args
        .project
        .clone()
        .unwrap_or_else(|| manifest::slugify(&name));

    let mut m = match &source {
        Some(src) => Manifest {
            name: name.clone(),
            project: project.clone(),
            like: Some(src.manifest.project.clone()),
            created: Some(today()),
            boards: src.manifest.boards.clone(),
            parts: src.manifest.parts.clone(),
            bom: src.manifest.bom.clone(),
            refs: src.manifest.refs.clone(),
            deny_terms: src.manifest.deny_terms.clone(),
            notes: src.manifest.notes.clone(),
        },
        None => Manifest {
            name: name.clone(),
            project: project.clone(),
            created: Some(today()),
            ..Default::default()
        },
    };
    for b in &args.boards {
        Manifest::push_unique(&mut m.boards, b);
    }
    for p in &args.parts {
        Manifest::push_unique(&mut m.parts, p);
    }
    let found = Found { dir, manifest: m };
    found.save()?;
    register(ctx, &found)?;

    if ctx.json() {
        output::json(&json!({
            "ok": true, "action": "created", "dir": found.dir, "file": found.path(),
            "like": source.as_ref().map(|s| s.manifest.project.clone()),
            "manifest": found.manifest,
        }));
        return Ok(());
    }
    match &source {
        Some(src) => {
            println!(
                "{} created {} {} {} · {}",
                render::good("✓"),
                render::bold(&found.manifest.name),
                render::dim("like"),
                render::bold(&src.manifest.name),
                render::dim(&found.path().display().to_string())
            );
            let sm = &src.manifest;
            println!(
                "  carried over: {} board{}, {} part{}, {} bom line{}, {} ref{}",
                sm.boards.len(),
                if sm.boards.len() == 1 { "" } else { "s" },
                sm.parts.len(),
                if sm.parts.len() == 1 { "" } else { "s" },
                sm.bom.len(),
                if sm.bom.len() == 1 { "" } else { "s" },
                sm.refs.len(),
                if sm.refs.len() == 1 { "" } else { "s" },
            );
            if !sm.boards.is_empty() {
                println!("  boards  {}", sm.boards.join(" "));
            }
            println!(
                "{}",
                render::dim(&format!(
                    "next: cd {} · sarg bom check (what to order) · sarg preflight (what sarg knows) · sarg ask --mine {}",
                    found.dir.display(),
                    sm.project
                ))
            );
        }
        None => report_written(ctx, &found, "created"),
    }
    Ok(())
}

fn ls(ctx: &mut Ctx) -> Result<()> {
    let mut rows: Vec<Value> = Vec::new();
    for (name, dir) in &ctx.cfg.projects {
        let d = Path::new(dir);
        let m = manifest::find(Some(d)).ok();
        rows.push(json!({
            "name": name,
            "dir": dir,
            "present": m.is_some(),
            "project": m.as_ref().map(|f| f.manifest.project.clone()),
            "like": m.as_ref().and_then(|f| f.manifest.like.clone()),
            "boards": m.as_ref().map(|f| f.manifest.boards.clone()).unwrap_or_default(),
        }));
    }
    if ctx.json() {
        output::json(&rows);
        return Ok(());
    }
    if rows.is_empty() {
        println!(
            "{}",
            render::dim("no projects registered — sarg project init in a project directory")
        );
        return Ok(());
    }
    for r in &rows {
        let present = r.get("present").and_then(|b| b.as_bool()) == Some(true);
        let mut bits = vec![render::bold(s(r, "name"))];
        if present {
            bits.push(format!("project {}", s(r, "project")));
            let boards = render::list(r, "boards");
            if !boards.is_empty() {
                bits.push(render::dim(&format!("boards {}", boards.join(" "))));
            }
            if !s(r, "like").is_empty() {
                bits.push(render::dim(&format!("like {}", s(r, "like"))));
            }
        } else {
            bits.push(render::warn("manifest missing"));
        }
        bits.push(render::dim(s(r, "dir")));
        println!("{}", bits.join(" · "));
    }
    Ok(())
}
