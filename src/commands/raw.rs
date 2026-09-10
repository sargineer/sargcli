use super::Ctx;
use crate::cli::RawArgs;
use crate::client::error_from;
use crate::error::{Result, SargError};
use crate::output;

pub fn run(ctx: &mut Ctx, args: &RawArgs) -> Result<()> {
    let method = args.method.to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD") {
        return Err(SargError::usage(format!("unsupported method `{}`", args.method)));
    }
    if !args.path.starts_with('/') {
        return Err(SargError::usage("path must start with '/'"));
    }
    let body = match &args.data {
        None => None,
        Some(d) if d == "@-" => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
            Some(s)
        }
        Some(d) if d.starts_with('@') => Some(std::fs::read_to_string(&d[1..])?),
        Some(d) => Some(d.clone()),
    };
    let mut headers: Vec<(String, String)> = args
        .headers
        .iter()
        .filter_map(|h| {
            let (k, v) = h.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    if let Some(b) = &body {
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
            let ct = if serde_json::from_str::<serde_json::Value>(b).is_ok() {
                "application/json"
            } else {
                "text/plain"
            };
            headers.push(("Content-Type".into(), ct.into()));
        }
    }
    let r = ctx
        .client
        .send(&method, &args.path, body.as_deref(), &headers)?;
    if ctx.cli.debug {
        eprintln!("sarg: HTTP {}", r.status);
    }
    if ctx.json() {
        output::json(&serde_json::json!({
            "status": r.status,
            "content_type": r.content_type,
            "body": r.json().unwrap_or(serde_json::Value::String(r.body.clone())),
        }));
    } else if !r.body.is_empty() {
        println!("{}", r.body.trim_end());
    } else {
        eprintln!("sarg: HTTP {} (empty body)", r.status);
    }
    if r.status >= 400 {
        return Err(error_from(&r));
    }
    Ok(())
}
