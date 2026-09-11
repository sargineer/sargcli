//! The command tree. Verbs read as asking sarg to do things.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "sarg",
    version,
    about = "Ask sarg, tell sarg: the command line for sargineer.com",
    long_about = "sarg is the command-line interface to sargineer.com — shared engineering \
memory for builders and their agents: lessons (symptom → cause → fix), the parts you \
own, and CAD models.\n\nEverything you write lands private. Publishing is your own act.",
    propagate_version = true,
    disable_help_subcommand = true,
    after_help = "ask sarg:   ask · show · part · tag · vendor · id · preflight · cad\n\
build:      project (init · new --like · add · show · ls) · bom check · stock\n\
plumbing:   auth · api · changelog · doc · raw · config · init · env"
)]
pub struct Cli {
    /// Server base URL (default: the config file, then https://sargineer.com)
    #[arg(long, global = true, value_name = "URL")]
    pub url: Option<String>,

    /// Bearer token for this call only; normally it lives in ~/.sargineer/config.toml
    #[arg(long, global = true, value_name = "TOKEN")]
    pub token: Option<String>,

    /// Talk to a named server for this call (`sarg env`): prod, or one you defined
    #[arg(long, global = true, value_name = "NAME")]
    pub env: Option<String>,

    /// Machine-readable JSON on stdout
    #[arg(long, global = true)]
    pub json: bool,

    /// Show requests on stderr (token redacted)
    #[arg(long, global = true)]
    pub debug: bool,

    /// Ignore cached server metadata
    #[arg(long, global = true)]
    pub no_cache: bool,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Search parts and lessons in one shot, ranked, compact
    Ask(AskArgs),

    /// One note, every field, unverified figures fenced
    Show {
        /// <handle>/<id>, or a bare id to look up
        r#ref: String,
    },

    /// A part: facts (with their confidence), models by role, related lessons
    Part {
        product: String,
        /// Print every fact
        #[arg(short, long)]
        verbose: bool,
    },

    /// Everything filed under one hw or sw tag
    Tag {
        tag: String,
        #[arg(short = 'n', long, default_value_t = 10)]
        n: usize,
    },

    /// A vendor's parts and the lessons about them
    Vendor {
        vendor: String,
        #[arg(short = 'n', long, default_value_t = 10)]
        n: usize,
    },

    /// What is this USB device, and what does sarg know about it? (vid:pid, /dev/ttyACM0 on Linux, or nothing to scan on Linux)
    Id { what: Option<String> },

    /// Before starting: what sarg knows about each board and intent, and the gaps
    Preflight(PreflightArgs),

    /// CAD models: list files by role, download them
    Cad {
        #[command(subcommand)]
        cmd: CadCmd,
    },

    /// Who you are on sargineer, sign-in links, tokens
    Auth {
        #[command(subcommand)]
        cmd: AuthCmd,
    },

    /// The server's own description of itself (cached 24 h)
    Api(ApiArgs),

    /// What changed on the server, newest first
    Changelog {
        /// Version or date to start from (default: the version you last saw)
        #[arg(long, value_name = "VERSION|DATE")]
        since: Option<String>,
        /// Everything, ignoring the version you last saw
        #[arg(long)]
        all: bool,
    },

    /// Print one of the server's guides
    Doc {
        #[arg(value_enum)]
        name: DocName,
    },

    /// Call any endpoint directly
    Raw(RawArgs),

    /// Read or change the local configuration
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },

    /// Record this machine and your token, then check the account
    Init(InitArgs),

    /// Which server sarg talks to: show, switch (`sarg env dev`), or define one
    Env(EnvArgs),

    /// Write a lesson (default), or list/edit/remove/publish your own
    Lesson {
        #[command(subcommand)]
        cmd: LessonCmd,
    },

    /// File something wrong with sargineer or a vendor (kind: issue)
    Issue(NoteArgs),

    /// Record how-you-should-work guidance (kind: feedback)
    Feedback(NoteArgs),

    /// Upload notes written while offline (~/.sargineer/pending)
    Sync,

    /// Should a command be preceded by a sarg search? (for shell wrappers/hooks)
    Guard {
        /// The command about to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },

    /// One line of sarg context for a prompt, or nothing (for hooks)
    Hint {
        /// The prompt text
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        text: Vec<String>,
    },

    /// Hook adapters and installation
    Hook {
        #[command(subcommand)]
        cmd: HookCmd,
    },

    /// The project manifest (sarg.yaml): boards, parts, BOM, refs — the unit of reuse
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },

    /// Bill of materials: on hand vs to buy, from the manifest
    Bom {
        #[command(subcommand)]
        cmd: BomCmd,
    },

    /// What is on hand (~/.sargineer/stock.yaml), by product id
    Stock {
        #[command(subcommand)]
        cmd: StockCmd,
    },

    /// Check the install: token, config, stale pointers, hooks, server drift
    Doctor,

    /// What sarg has been asked, from the local journal
    Stats {
        /// Only this session
        #[arg(long)]
        session: Option<String>,
    },

    /// Emit a SKILL.md that delegates to this binary
    Skill {
        /// Write to a file instead of stdout
        #[arg(short, long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)] // NoteArgs is parsed once; boxing buys nothing
pub enum LessonCmd {
    /// Compose and post a lesson (private until you publish it)
    New(NoteArgs),
    /// List your own notes
    Ls {
        #[arg(long, value_name = "PRODUCT")]
        hw: Option<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        n: usize,
    },
    /// Edit one of your notes in $EDITOR (keeps id and publish state)
    Edit {
        r#ref: String,
        /// Allow home paths, emails or private IPs to leave the machine (never lifts deny_terms)
        #[arg(long)]
        allow_pii: bool,
    },
    /// Delete one of your notes
    Rm { r#ref: String },
    /// Make a note public — your act; needs a TTY confirm or --user-authorized
    Publish {
        r#ref: String,
        #[arg(long)]
        user_authorized: bool,
    },
    /// Remove a note from public
    Hide {
        r#ref: String,
        #[arg(long)]
        user_authorized: bool,
    },
}

/// Fields shared by lesson/issue/feedback creation.
#[derive(Args, Debug, Default)]
pub struct NoteArgs {
    /// Read the note from a JSON or TOML file (@- for stdin)
    #[arg(short = 'f', long, value_name = "FILE")]
    pub file: Option<String>,
    /// Open $EDITOR on a filled template
    #[arg(long)]
    pub edit: bool,
    /// Validate and scan, but do not send
    #[arg(long)]
    pub dry_run: bool,
    /// Allow home paths, emails or private IPs to leave the machine (never lifts deny_terms)
    #[arg(long)]
    pub allow_pii: bool,

    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub symptom: Option<String>,
    #[arg(long)]
    pub cause: Option<String>,
    #[arg(long)]
    pub fix: Option<String>,
    #[arg(long)]
    pub setup: Option<String>,
    #[arg(long)]
    pub check: Option<String>,
    #[arg(long)]
    pub intent: Option<String>,
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
    /// issues only: whose problem this is (sargineer.com, or a vendor tag)
    #[arg(long)]
    pub about: Option<String>,
    #[arg(long)]
    pub body: Option<String>,
    /// hardware tag (repeatable, or comma-separated)
    #[arg(long)]
    pub hw: Vec<String>,
    /// software tag@version (repeatable)
    #[arg(long)]
    pub sw: Vec<String>,
    /// a runnable step (repeatable, in order)
    #[arg(long = "step")]
    pub steps: Vec<String>,
    /// an unverified figure, kept fenced (repeatable)
    #[arg(long = "unverified")]
    pub unverified: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum HookCmd {
    /// Run the Claude Code hook adapter for one event (reads hook JSON on stdin)
    Claude {
        #[arg(value_enum)]
        event: ClaudeEvent,
    },
    /// Add sarg's hooks to ~/.claude/settings.json
    Install {
        /// Print what would change without writing
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove sarg's hooks from ~/.claude/settings.json
    Uninstall,
    /// Are the hooks installed?
    Status,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ClaudeEvent {
    /// PreToolUse on Bash — the flash/bring-up guard
    Pretooluse,
    /// UserPromptSubmit — inject one line of sarg context
    Userpromptsubmit,
    /// Stop — nudge to record a lesson
    Stop,
}

#[derive(Args, Debug)]
pub struct AskArgs {
    /// Words to search: the verbatim error first, then the board, then the intent
    #[arg(required_unless_present = "hw")]
    pub query: Vec<String>,
    /// Only lessons about this part (product id), newest first
    #[arg(long, value_name = "PRODUCT")]
    pub hw: Option<String>,
    /// How many lessons
    #[arg(short = 'n', long, default_value_t = 8)]
    pub n: usize,
    /// Full lessons instead of compact hits
    #[arg(short, long)]
    pub verbose: bool,
    /// Only my own notes
    #[arg(long)]
    pub mine: bool,
}

#[derive(Args, Debug)]
pub struct PreflightArgs {
    /// Boards or parts in play (default: the boards in the project's sarg.yaml)
    pub boards: Vec<String>,
    /// A risky thing you are about to do (repeatable)
    #[arg(short, long = "intent", value_name = "TEXT")]
    pub intents: Vec<String>,
    /// Lessons per query
    #[arg(short = 'n', long, default_value_t = 5)]
    pub n: usize,
}

#[derive(Subcommand, Debug)]
pub enum CadCmd {
    /// Files of a model, grouped by role
    Ls {
        /// <handle>/<product>, or <product> to see every model of it
        r#ref: String,
    },
    /// Download a model's files
    Get {
        /// <handle>/<product>, or <product> when only one model exists
        r#ref: String,
        /// Only files with this role: print, cad, source, image
        #[arg(long)]
        role: Option<String>,
        /// Only these file names (repeatable)
        #[arg(long = "file", value_name = "NAME")]
        files: Vec<String>,
        /// Directory to write into (default ./<product>)
        #[arg(short, long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum AuthCmd {
    /// Am I signed in, and as whom?
    Status,
    /// Mint a one-time web sign-in URL (valid ten minutes, works once)
    LoginLink {
        /// Open it in the browser
        #[arg(long)]
        open: bool,
    },
    /// Mint a fresh API token (shown once)
    TokenMint,
}

#[derive(Args, Debug)]
pub struct ApiArgs {
    /// Fetch again even if the cache is fresh
    #[arg(long)]
    pub refresh: bool,
    /// Print note_fields (the validator's own description) instead of endpoints
    #[arg(long)]
    pub fields: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum DocName {
    Search,
    Share,
    Start,
    Skill,
}

impl DocName {
    pub fn path(self) -> &'static str {
        match self {
            DocName::Search => "/search.md",
            DocName::Share => "/share.md",
            DocName::Start => "/start.md",
            DocName::Skill => "/skill",
        }
    }
}

#[derive(Args, Debug)]
pub struct RawArgs {
    /// GET, POST, PUT or DELETE
    pub method: String,
    /// Path starting with '/', query string allowed
    pub path: String,
    /// Request body: inline text, or @file to read one
    #[arg(short = 'd', long, value_name = "BODY|@FILE")]
    pub data: Option<String>,
    /// Extra header, repeatable: -H 'Accept: text/markdown'
    #[arg(short = 'H', long = "header", value_name = "K: V")]
    pub headers: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCmd {
    /// Where the config file lives
    Path,
    /// Print the whole config (token masked) or one key
    Get {
        key: Option<String>,
        /// Print the token in clear
        #[arg(long)]
        reveal: bool,
    },
    /// Set a key: url, token, default_project, host_tags, deny_terms, project_aliases.<name>, projects.<name>
    Set { key: String, value: String },
    /// Remove a key
    Unset { key: String },
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Do not probe for host tags
    #[arg(long)]
    pub no_probe: bool,
    /// Do not contact the server afterwards
    #[arg(long)]
    pub offline: bool,
}

/// `sarg env` shows the servers; `sarg env NAME` switches; with `--url`
/// (and usually `--token`) it defines or updates NAME first. `prod` is the
/// top-level url/token in config.toml and cannot be removed.
#[derive(Args, Debug)]
pub struct EnvArgs {
    /// Env to switch to, define, or remove (omit to list)
    pub name: Option<String>,
    /// Server base URL for this env
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// Bearer token for this env (stored in config.toml, never printed)
    #[arg(long, value_name = "TOKEN")]
    pub token: Option<String>,
    /// Define or update without switching to it
    #[arg(long)]
    pub no_switch: bool,
    /// Remove the env (switches back to prod if it was active)
    #[arg(long)]
    pub rm: bool,
}

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// Start a manifest here (or in DIR); registers the project on this machine
    Init(ProjectInitArgs),
    /// The manifest, with what sarg knows about each part
    Show {
        /// Project directory (default: here or above)
        dir: Option<PathBuf>,
        /// Do not contact the server
        #[arg(long)]
        offline: bool,
    },
    /// Add boards, parts, BOM lines or refs to the manifest
    Add(ProjectAddArgs),
    /// Start a project from one that already works: boards, parts, BOM and refs carried over
    New(ProjectNewArgs),
    /// Projects registered on this machine
    Ls,
}

#[derive(Args, Debug)]
pub struct ProjectInitArgs {
    /// Directory to start in (default: here)
    pub dir: Option<PathBuf>,
    /// Display name (default: the directory name)
    #[arg(long)]
    pub name: Option<String>,
    /// The `project` slug sent on lessons (default: slug of the name)
    #[arg(long)]
    pub project: Option<String>,
    /// A board by product id (repeatable)
    #[arg(long = "board", value_name = "PRODUCT")]
    pub boards: Vec<String>,
    /// Another part by product id (repeatable)
    #[arg(long = "part", value_name = "PRODUCT")]
    pub parts: Vec<String>,
    /// Overwrite an existing manifest
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct ProjectAddArgs {
    /// Product ids to add (parts by default; --board for boards)
    pub ids: Vec<String>,
    /// Add the ids as boards (what ask/preflight scope to)
    #[arg(long)]
    pub board: bool,
    /// Also put each id on the BOM with this quantity
    #[arg(long, value_name = "N")]
    pub qty: Option<u32>,
    /// A free-text BOM line (screws, wire, filament); pairs with --qty
    #[arg(long, value_name = "TEXT")]
    pub item: Option<String>,
    /// A note on the BOM line
    #[arg(long)]
    pub note: Option<String>,
    /// A note or model this project depends on, <handle>/<id> (repeatable)
    #[arg(long = "ref", value_name = "HANDLE/ID")]
    pub refs: Vec<String>,
    /// Project directory (default: here or above)
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ProjectNewArgs {
    /// Directory for the new project (created if missing)
    pub dir: PathBuf,
    /// Clone from: a directory with sarg.yaml, or a registered project name/slug
    #[arg(long, value_name = "PROJECT|DIR")]
    pub like: Option<String>,
    /// Display name (default: the directory name)
    #[arg(long)]
    pub name: Option<String>,
    /// The `project` slug sent on lessons (default: slug of the name)
    #[arg(long)]
    pub project: Option<String>,
    /// Extra board by product id (repeatable)
    #[arg(long = "board", value_name = "PRODUCT")]
    pub boards: Vec<String>,
    /// Extra part by product id (repeatable)
    #[arg(long = "part", value_name = "PRODUCT")]
    pub parts: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum BomCmd {
    /// On hand vs to buy: stock file first, then what sargineer says you own
    Check {
        /// Project directory (default: here or above)
        dir: Option<PathBuf>,
        /// Stock file only; do not contact the server
        #[arg(long)]
        offline: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum StockCmd {
    /// Everything tracked
    Ls,
    /// Record a quantity on hand
    Set {
        /// Product id, or an item name (slugged)
        id: String,
        qty: u32,
        /// Bin or location
        #[arg(long = "where", value_name = "BIN")]
        location: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Stop tracking one
    Rm { id: String },
}

impl Cli {
    /// Short verb name for the journal.
    pub fn verb(&self) -> String {
        match &self.cmd {
            Cmd::Ask(_) => "ask".into(),
            Cmd::Show { .. } => "show".into(),
            Cmd::Part { .. } => "part".into(),
            Cmd::Tag { .. } => "tag".into(),
            Cmd::Vendor { .. } => "vendor".into(),
            Cmd::Id { .. } => "id".into(),
            Cmd::Preflight(_) => "preflight".into(),
            Cmd::Cad { cmd } => match cmd {
                CadCmd::Ls { .. } => "cad.ls",
                CadCmd::Get { .. } => "cad.get",
            }
            .into(),
            Cmd::Auth { cmd } => match cmd {
                AuthCmd::Status => "auth.status",
                AuthCmd::LoginLink { .. } => "auth.login-link",
                AuthCmd::TokenMint => "auth.token-mint",
            }
            .to_string(),
            Cmd::Api(_) => "api".into(),
            Cmd::Changelog { .. } => "changelog".into(),
            Cmd::Doc { name } => format!("doc.{name:?}").to_lowercase(),
            Cmd::Raw(a) => format!("raw.{}", a.method.to_lowercase()),
            Cmd::Config { cmd } => match cmd {
                ConfigCmd::Path => "config.path",
                ConfigCmd::Get { .. } => "config.get",
                ConfigCmd::Set { .. } => "config.set",
                ConfigCmd::Unset { .. } => "config.unset",
            }
            .to_string(),
            Cmd::Init(_) => "init".into(),
            Cmd::Env(_) => "env".into(),
            Cmd::Lesson { cmd } => match cmd {
                LessonCmd::New(_) => "lesson.new",
                LessonCmd::Ls { .. } => "lesson.ls",
                LessonCmd::Edit { .. } => "lesson.edit",
                LessonCmd::Rm { .. } => "lesson.rm",
                LessonCmd::Publish { .. } => "lesson.publish",
                LessonCmd::Hide { .. } => "lesson.hide",
            }
            .into(),
            Cmd::Issue(_) => "issue.new".into(),
            Cmd::Feedback(_) => "feedback.new".into(),
            Cmd::Sync => "sync".into(),
            Cmd::Guard { .. } => "guard".into(),
            Cmd::Hint { .. } => "hint".into(),
            Cmd::Hook { cmd } => match cmd {
                HookCmd::Claude { event } => match event {
                    ClaudeEvent::Pretooluse => "hook.claude.pretooluse",
                    ClaudeEvent::Userpromptsubmit => "hook.claude.userpromptsubmit",
                    ClaudeEvent::Stop => "hook.claude.stop",
                },
                HookCmd::Install { .. } => "hook.install",
                HookCmd::Uninstall => "hook.uninstall",
                HookCmd::Status => "hook.status",
            }
            .into(),
            Cmd::Project { cmd } => match cmd {
                ProjectCmd::Init(_) => "project.init",
                ProjectCmd::Show { .. } => "project.show",
                ProjectCmd::Add(_) => "project.add",
                ProjectCmd::New(_) => "project.new",
                ProjectCmd::Ls => "project.ls",
            }
            .into(),
            Cmd::Bom { cmd } => match cmd {
                BomCmd::Check { .. } => "bom.check",
            }
            .into(),
            Cmd::Stock { cmd } => match cmd {
                StockCmd::Ls => "stock.ls",
                StockCmd::Set { .. } => "stock.set",
                StockCmd::Rm { .. } => "stock.rm",
            }
            .into(),
            Cmd::Doctor => "doctor".into(),
            Cmd::Stats { .. } => "stats".into(),
            Cmd::Skill { .. } => "skill".into(),
        }
    }
}
