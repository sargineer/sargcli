//! Dispatch. Each verb gets a `Ctx` with the loaded config, the resolved
//! server settings and a client; nothing here touches the network itself.

pub mod api_cmd;
pub mod ask;
pub mod auth;
pub mod bom;
pub mod cad;
pub mod changelog;
pub mod config_cmd;
pub mod doc;
pub mod doctor;
pub mod env_cmd;
pub mod hook;
pub mod id;
pub mod init;
pub mod part;
pub mod preflight;
pub mod project;
pub mod raw;
pub mod record;
pub mod show;
pub mod skill;
pub mod stats;
pub mod stock_cmd;
pub mod tag;

use crate::cli::{Cli, Cmd};
use crate::client::Client;
use crate::config::{self, Config, Paths, Resolved};
use crate::error::Result;
use crate::journal;
use crate::render;

pub struct Ctx<'a> {
    pub cli: &'a Cli,
    pub paths: Paths,
    pub cfg: Config,
    pub resolved: Resolved,
    pub client: Client,
}

impl<'a> Ctx<'a> {
    pub fn new(cli: &'a Cli, paths: Paths) -> Result<Self> {
        let cfg = Config::load(&paths)?;
        let resolved = config::resolve(cli, &cfg)?;
        // A named env gets its own cache and spool; prod keeps the classic paths.
        let paths = paths.for_env(&resolved.env);
        if !resolved.is_prod() {
            journal::note("env", resolved.env.clone());
            // Say so on stderr for every verb an agent or a person reads, so
            // a dev answer is never mistaken for sargineer.com's. Hooks feed
            // their stdout straight into the agent and stay quiet.
            if !matches!(cli.cmd, Cmd::Hook { .. } | Cmd::Guard { .. } | Cmd::Env(_)) {
                eprintln!(
                    "{}",
                    render::warn(&format!("sarg: env {} · {}", resolved.env, resolved.url))
                );
            }
        }
        let client = Client::new(&resolved, cli.debug);
        Ok(Ctx {
            cli,
            paths,
            cfg,
            resolved,
            client,
        })
    }

    pub fn json(&self) -> bool {
        self.cli.json
    }
}

/// Returns the process exit code. Most verbs succeed with 0; guard/hook
/// map their advisory decision onto a code the caller can branch on.
pub fn run(cli: &Cli, paths: Paths) -> Result<i32> {
    let mut ctx = Ctx::new(cli, paths)?;
    let zero = |r: Result<()>| r.map(|_| 0);
    match &cli.cmd {
        Cmd::Ask(args) => zero(ask::run(&mut ctx, args)),
        Cmd::Show { r#ref } => zero(show::run(&mut ctx, r#ref)),
        Cmd::Part { product, verbose } => zero(part::run(&mut ctx, product, *verbose)),
        Cmd::Tag { tag, n } => zero(tag::run(&mut ctx, tag::Kind::Tag, tag, *n)),
        Cmd::Vendor { vendor, n } => zero(tag::run(&mut ctx, tag::Kind::Vendor, vendor, *n)),
        Cmd::Id { what } => zero(id::run(&mut ctx, what.as_deref())),
        Cmd::Preflight(args) => zero(preflight::run(&mut ctx, args)),
        Cmd::Cad { cmd } => zero(cad::run(&mut ctx, cmd)),
        Cmd::Auth { cmd } => zero(auth::run(&mut ctx, cmd)),
        Cmd::Api(args) => zero(api_cmd::run(&mut ctx, args)),
        Cmd::Changelog { since, all } => zero(changelog::run(&mut ctx, since.as_deref(), *all)),
        Cmd::Doc { name } => zero(doc::run(&mut ctx, *name)),
        Cmd::Raw(args) => zero(raw::run(&mut ctx, args)),
        Cmd::Config { cmd } => zero(config_cmd::run(&mut ctx, cmd)),
        Cmd::Init(args) => zero(init::run(&mut ctx, args)),
        Cmd::Env(args) => zero(env_cmd::run(&mut ctx, args)),
        Cmd::Lesson { cmd } => zero(record::lesson(&mut ctx, cmd)),
        Cmd::Issue(args) => zero(record::issue(&mut ctx, args)),
        Cmd::Feedback(args) => zero(record::feedback(&mut ctx, args)),
        Cmd::Sync => zero(record::sync(&mut ctx)),
        Cmd::Guard { command } => hook::guard(&mut ctx, command),
        Cmd::Hint { text } => hook::hint(&mut ctx, text),
        Cmd::Hook { cmd } => hook::run(&mut ctx, cmd),
        Cmd::Project { cmd } => zero(project::run(&mut ctx, cmd)),
        Cmd::Bom { cmd } => zero(bom::run(&mut ctx, cmd)),
        Cmd::Stock { cmd } => zero(stock_cmd::run(&mut ctx, cmd)),
        Cmd::Doctor => zero(doctor::run(&mut ctx)),
        Cmd::Stats { session } => zero(stats::run(&mut ctx, session.as_deref())),
        Cmd::Skill { out } => zero(skill::run(&mut ctx, out)),
    }
}
