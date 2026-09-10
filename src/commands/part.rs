//! `sarg part <product>`: the part page, with the confidence of its facts
//! made visible — the 08-21 case was built on numbers a page had flagged as
//! estimates in free text nobody read.

use serde_json::Value;

use super::Ctx;
use crate::error::Result;
use crate::output;
use crate::render::{self, list, s, status_badge, truncate, u};

const ESTIMATE_WORDS: [&str; 8] = [
    "estimat", "verify", "unmeasured", "caliper", "guess", "assum", "not measured", "placeholder",
];

/// Free-text facts that admit to being guesses.
pub fn estimate_flags(facts: &Value) -> Vec<String> {
    let mut flags = Vec::new();
    let mut scan = |label: &str, text: &str| {
        let t = text.to_lowercase();
        if ESTIMATE_WORDS.iter().any(|w| t.contains(w)) {
            flags.push(format!("{label}: {}", truncate(text, 140)));
        }
    };
    scan("confidence", s(facts, "confidence"));
    if let Some(b) = facts.get("body") {
        scan("body", s(b, "notes"));
    }
    if let Some(m) = facts.get("mounting") {
        scan("mounting", s(m, "notes"));
    }
    if let Some(p) = facts.get("ports") {
        scan("ports", &p.to_string());
    }
    if let Some(srcs) = facts.get("sources").and_then(|x| x.as_array()) {
        for src in srcs {
            scan("source", s(src, "facts"));
        }
    }
    if let Some(cad) = facts.get("cad") {
        if cad.get("verified").and_then(|v| v.as_bool()) == Some(false) {
            flags.push(format!(
                "cad: {} model, verified: false",
                s(cad, "fidelity")
            ));
        }
    }
    flags
}

pub fn run(ctx: &mut Ctx, product: &str, verbose: bool) -> Result<()> {
    let product = product.trim().trim_start_matches("/p/");
    let v = ctx.client.get_json(&format!("/p/{product}"))?;
    if ctx.json() {
        output::json(&v);
        return Ok(());
    }
    let w = render::width();
    let part = v.get("part").cloned().unwrap_or(Value::Null);
    let facts = v.get("facts").cloned().unwrap_or(Value::Null);

    // Header
    let mut head = vec![render::bold(product)];
    let name = if s(&part, "name").is_empty() {
        s(&facts, "name")
    } else {
        s(&part, "name")
    };
    if !name.is_empty() {
        head.push(name.to_string());
    }
    println!("{}", head.join(" · "));
    let mut meta = Vec::new();
    for k in ["vendor", "sku", "category", "kind"] {
        if !s(&part, k).is_empty() {
            meta.push(format!("{k} {}", s(&part, k)));
        }
    }
    let owned = u(&part, "owned_qty").max(u(&facts, "qty_owned"));
    if owned > 0 {
        meta.push(render::good(&format!("owned ×{owned}")));
    }
    let aka = list(&facts, "aka");
    if !aka.is_empty() {
        meta.push(format!("aka {}", aka.join(", ")));
    }
    if !meta.is_empty() {
        println!("{}", render::dim(&meta.join(" · ")));
    }
    if !s(&part, "url").is_empty() {
        println!("{}", render::dim(s(&part, "url")));
    }

    // Facts summary
    if !facts.is_null() {
        println!();
        if let Some(size) = facts.get("body").and_then(|b| b.get("size_mm")) {
            println!("size_mm     {size}  {}", render::dim(s(facts.get("body").unwrap(), "outline")));
        }
        if !s(&facts, "frame").is_empty() {
            println!("frame       {}", truncate(s(&facts, "frame"), w.saturating_sub(12)));
        }
        if let Some(m) = facts.get("mounting") {
            let holes = m.get("holes").and_then(|h| h.as_array()).map(|a| a.len()).unwrap_or(0);
            println!(
                "mounting    {} hole{} {}",
                holes,
                if holes == 1 { "" } else { "s" },
                render::dim(&truncate(s(m, "notes"), w.saturating_sub(24)))
            );
        }
        if let Some(e) = facts.get("electrical") {
            let mut bits = Vec::new();
            if !s(e, "vin").is_empty() {
                bits.push(format!("vin {}", s(e, "vin")));
            }
            if let Some(lv) = e.get("logic_v") {
                bits.push(format!("logic {lv} V"));
            }
            let protos = list(e, "protocols");
            if !protos.is_empty() {
                bits.push(protos.join("/"));
            }
            if !bits.is_empty() {
                println!("electrical  {}", truncate(&bits.join(" · "), w.saturating_sub(12)));
            }
        }
        let tags = list(&facts, "hw_tags");
        if !tags.is_empty() {
            println!("hw_tags     {}", tags.join(" "));
        }
        let flags = estimate_flags(&facts);
        if flags.is_empty() {
            if !s(&facts, "confidence").is_empty() {
                println!("confidence  {}", truncate(s(&facts, "confidence"), w.saturating_sub(12)));
            }
        } else {
            println!("{}", render::warn("⚠ facts partly estimated — caliper before anything fit-critical:"));
            for f in flags {
                println!("{}", render::warn(&format!("  {f}")));
            }
        }
        if verbose {
            println!();
            println!("{}", serde_json::to_string_pretty(&facts)?);
        }
    }

    // Models
    if let Some(models) = v.get("models").and_then(|m| m.as_array()) {
        println!();
        if models.is_empty() {
            println!("{}", render::dim("models      none — the queue may want one"));
        }
        for m in models {
            let files = m.get("files").and_then(|f| f.as_array()).cloned().unwrap_or_default();
            // /p lists files as names or as {name, role} objects depending on version.
            let roles: Vec<String> = files
                .iter()
                .map(|f| match f {
                    Value::String(name) => name.clone(),
                    obj if !s(obj, "role").is_empty() => {
                        format!("{} ({})", s(obj, "name"), s(obj, "role"))
                    }
                    obj => s(obj, "name").to_string(),
                })
                .collect();
            println!(
                "model       {} · {} · {} · {}",
                render::bold(&format!("{}/{product}", s(m, "handle"))),
                s(m, "fidelity"),
                if m.get("verified").and_then(|x| x.as_bool()) == Some(true) {
                    render::good("verified")
                } else {
                    render::warn("unverified")
                },
                s(m, "visibility")
            );
            if !roles.is_empty() {
                println!("            {}", render::dim(&truncate(&roles.join(", "), w.saturating_sub(12))));
            }
        }
    }

    // Links to made parts
    if let Some(links) = v.get("links") {
        for k in ["cases", "assemblies", "fits", "members"] {
            let Some(items) = links.get(k).and_then(|x| x.as_array()) else {
                continue;
            };
            if items.is_empty() {
                continue;
            }
            println!("{}", render::dim(k));
            for it in items {
                match it {
                    Value::String(id) => println!("  {}", render::bold(id)),
                    obj => {
                        let id = if s(obj, "product").is_empty() {
                            s(obj, "id")
                        } else {
                            s(obj, "product")
                        };
                        println!(
                            "  {} {}",
                            render::bold(id),
                            render::dim(&format!(
                                "{}{}",
                                if s(obj, "kind").is_empty() {
                                    String::new()
                                } else {
                                    format!("{} · ", s(obj, "kind"))
                                },
                                truncate(s(obj, "name"), w.saturating_sub(id.len() + 14))
                            ))
                        );
                    }
                }
            }
        }
    }

    // Bring-up digest, when the server has assembled one
    if let Some(bring) = v.get("digest").and_then(|d| d.get("bringup")) {
        if let Some(steps) = bring.get("steps").and_then(|x| x.as_array()) {
            println!();
            println!("{} {}", render::bold("bring-up"), render::dim(s(bring, "intro")));
            for (i, st) in steps.iter().enumerate() {
                println!(
                    "  {}. {} {}",
                    i + 1,
                    s(st, "title"),
                    render::dim(&format!("({})", s(st, "ref")))
                );
            }
        }
    }

    // Related lessons
    if let Some(exact) = v
        .get("related")
        .and_then(|r| r.get("notes"))
        .and_then(|n| n.get("exact"))
        .and_then(|e| e.as_array())
    {
        println!();
        println!(
            "{} {}",
            render::bold("lessons"),
            render::dim(&format!("{} about this part", exact.len()))
        );
        for n in exact.iter().take(8) {
            println!(
                "  {} {} {}",
                status_badge(s(n, "status")),
                render::bold(&format!("{}/{}", s(n, "handle"), s(n, "id"))),
                render::dim(&truncate(s(n, "title"), w.saturating_sub(30)))
            );
        }
        if exact.len() > 8 {
            println!(
                "{}",
                render::dim(&format!("  … sarg ask --hw {product} for all {}", exact.len()))
            );
        }
    }
    Ok(())
}
