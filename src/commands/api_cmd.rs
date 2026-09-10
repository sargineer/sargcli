use super::Ctx;
use crate::api;
use crate::cli::ApiArgs;
use crate::error::Result;
use crate::output;

pub fn run(ctx: &mut Ctx, args: &ApiArgs) -> Result<()> {
    let force = args.refresh || ctx.cli.no_cache;
    let paths = ctx.paths.clone();
    let env = ctx.resolved.env.clone();
    let info = api::load(&ctx.client, &paths, &mut ctx.cfg, &env, force)?;
    if ctx.json() {
        if args.fields {
            output::json(&info.note_fields);
        } else {
            output::json(&info.raw);
        }
        return Ok(());
    }
    let source = if info.fresh {
        "fetched now".to_string()
    } else {
        format!("cached {}", human_age(info.cache_age_secs))
    };
    println!(
        "sargineer {} · updated {} · {} · {}",
        info.version, info.updated, source, ctx.resolved.url
    );
    if args.fields {
        println!("{}", serde_json::to_string_pretty(&info.note_fields)?);
        return Ok(());
    }
    println!();
    for e in &info.endpoints {
        println!("  {e}");
    }
    println!();
    println!("sarg api --fields  for the note field rules · sarg raw <METHOD> <path>  to call one");
    Ok(())
}

fn human_age(secs: u64) -> String {
    match secs {
        s if s < 90 => format!("{s}s ago"),
        s if s < 5400 => format!("{}m ago", s / 60),
        s => format!("{}h ago", s / 3600),
    }
}
