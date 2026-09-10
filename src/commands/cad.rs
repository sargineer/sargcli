//! `sarg cad ls|get`: model files grouped by the server's roles, and a
//! downloader that says plainly when a 401 means "sign in", not "gone".

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::Ctx;
use crate::cli::CadCmd;
use crate::error::{Result, SargError};
use crate::output;
use crate::render::{self, human_bytes, s, u};

const ROLE_ORDER: [&str; 5] = ["print", "cad", "source", "image", ""];

fn role_label(r: &str) -> &'static str {
    match r {
        "print" => "print these",
        "cad" => "CAD geometry",
        "source" => "agent source",
        "image" => "images",
        _ => "other",
    }
}

/// `<handle>/<product>` → the model; a bare product resolves when exactly one model exists.
fn resolve(ctx: &Ctx, r#ref: &str) -> Result<(String, String, Value)> {
    let r = r#ref.trim().trim_start_matches("/m/").trim_start_matches('/');
    if let Some((h, p)) = r.split_once('/') {
        let v = ctx.client.get_json(&format!("/m/{h}/{p}"))?;
        return Ok((h.to_string(), p.to_string(), v));
    }
    let all = ctx.client.get_json(&format!("/models/{r}"))?;
    let models = all.get("models").and_then(|m| m.as_array()).cloned().unwrap_or_default();
    match models.len() {
        0 => Err(SargError::usage(format!("nobody has modelled `{r}` yet — sarg part {r}"))),
        1 => {
            let h = s(&models[0], "handle").to_string();
            let v = ctx.client.get_json(&format!("/m/{h}/{r}"))?;
            Ok((h, r.to_string(), v))
        }
        _ => Err(SargError::usage(format!(
            "{} models of `{r}` — pick one: {}",
            models.len(),
            models
                .iter()
                .map(|m| format!("{}/{r}", s(m, "handle")))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn files_of(v: &Value) -> Vec<Value> {
    v.get("files").and_then(|f| f.as_array()).cloned().unwrap_or_default()
}

fn print_model(handle: &str, product: &str, v: &Value) {
    println!(
        "{} · {} · {} · {}",
        render::bold(&format!("{handle}/{product}")),
        s(v, "fidelity"),
        if v.get("verified").and_then(|x| x.as_bool()) == Some(true) {
            render::good("verified")
        } else {
            render::warn("unverified")
        },
        s(v, "visibility")
    );
    let files = files_of(v);
    for role in ROLE_ORDER {
        let group: Vec<&Value> = files
            .iter()
            .filter(|f| {
                let r = s(f, "role");
                if role.is_empty() {
                    !ROLE_ORDER[..4].contains(&r)
                } else {
                    r == role
                }
            })
            .collect();
        if group.is_empty() {
            continue;
        }
        println!("  {}", render::dim(role_label(role)));
        for f in group {
            println!("    {:<40} {:>9}", s(f, "name"), human_bytes(u(f, "bytes")));
        }
    }
    if files.is_empty() {
        println!("  {}", render::dim("no files — declared but never uploaded"));
    }
}

pub fn run(ctx: &mut Ctx, cmd: &CadCmd) -> Result<()> {
    match cmd {
        CadCmd::Ls { r#ref } => ls(ctx, r#ref),
        CadCmd::Get {
            r#ref,
            role,
            files,
            out,
        } => get(ctx, r#ref, role.as_deref(), files, out.clone()),
    }
}

fn ls(ctx: &mut Ctx, r#ref: &str) -> Result<()> {
    let r = r#ref.trim().trim_start_matches("/m/").trim_start_matches('/');
    if !r.contains('/') {
        let all = ctx.client.get_json(&format!("/models/{r}"))?;
        if ctx.json() {
            output::json(&all);
            return Ok(());
        }
        let models = all.get("models").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        println!(
            "{} {} · {} model{}",
            render::bold(r),
            render::dim(s(&all, "name")),
            models.len(),
            if models.len() == 1 { "" } else { "s" }
        );
        for m in &models {
            println!();
            print_model(s(m, "handle"), r, m);
        }
        if models.is_empty() {
            println!("{}", render::dim("nobody has modelled it — sarg raw GET /queue lists what is wanted"));
        }
        return Ok(());
    }
    let (h, p, v) = resolve(ctx, r)?;
    if ctx.json() {
        output::json(&v);
        return Ok(());
    }
    print_model(&h, &p, &v);
    println!("{}", render::dim(&format!("sarg cad get {h}/{p} [--role print|cad|source] [-o dir]")));
    Ok(())
}

fn get(
    ctx: &mut Ctx,
    r#ref: &str,
    role: Option<&str>,
    only: &[String],
    out: Option<PathBuf>,
) -> Result<()> {
    let (h, p, v) = resolve(ctx, r#ref)?;
    let dir = out.unwrap_or_else(|| PathBuf::from(&p));
    let files: Vec<Value> = files_of(&v)
        .into_iter()
        .filter(|f| role.is_none_or(|r| s(f, "role") == r))
        .filter(|f| only.is_empty() || only.iter().any(|o| o == s(f, "name")))
        .collect();
    if files.is_empty() {
        return Err(SargError::usage(format!(
            "no files match{}{} — sarg cad ls {h}/{p}",
            role.map(|r| format!(" role {r}")).unwrap_or_default(),
            if only.is_empty() { String::new() } else { format!(" names {}", only.join(",")) }
        )));
    }
    if !ctx.client.has_token() {
        eprintln!(
            "{}",
            render::warn("sarg: signed out — geometry downloads answer 401 without a token")
        );
    }
    fs::create_dir_all(&dir)?;
    let mut saved = Vec::new();
    for f in &files {
        let name = s(f, "name");
        if name.contains("..") || name.contains('/') {
            return Err(SargError::Validation {
                message: format!("refusing suspicious file name `{name}`"),
                hint: None,
            });
        }
        let bytes = ctx
            .client
            .get_bytes(&format!("/m/{h}/{p}/f/{name}"))
            .map_err(|e| match e {
                SargError::Auth { message, .. } => SargError::Auth {
                    message: format!("{message} — the file exists; downloading geometry needs a token"),
                    hint: Some("sarg init --from-claude-settings · sarg config set token <token>".into()),
                },
                other => other,
            })?;
        let path = dir.join(name);
        fs::write(&path, &bytes)?;
        if !ctx.json() {
            println!(
                "{:<9} {:<8} {}",
                human_bytes(bytes.len() as u64),
                render::dim(s(f, "role")),
                path.display()
            );
        }
        saved.push(json!({ "name": name, "role": s(f, "role"), "bytes": bytes.len(), "path": path }));
    }
    if ctx.json() {
        output::json(&json!({ "handle": h, "product": p, "dir": dir, "files": saved }));
    } else {
        println!(
            "{}",
            render::dim(&format!("{} file{} → {}", saved.len(), if saved.len() == 1 { "" } else { "s" }, dir.display()))
        );
    }
    Ok(())
}
