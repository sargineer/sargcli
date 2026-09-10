//! `sarg skill`: emit a SKILL.md that delegates to this binary. It replaces
//! the curl-from-markdown skill with "run these verbs", so no session pays
//! to relearn the wire protocol. Carries the machine tags and deny terms
//! from config into plain prose. Prints to stdout unless `-o` is given.

use std::fs;
use std::path::PathBuf;

use super::Ctx;
use crate::error::Result;

fn body(ctx: &Ctx) -> String {
    let tags = if ctx.cfg.host_tags.is_empty() {
        "(run `sarg init` to fill these)".to_string()
    } else {
        ctx.cfg.host_tags.join(", ")
    };
    let deny = if ctx.cfg.deny_terms.is_empty() {
        "none set".to_string()
    } else {
        ctx.cfg.deny_terms.join(", ")
    };
    format!(
        r#"---
name: sarg
description: The user's engineering memory on sargineer.com — lessons (symptom → cause → fix), parts, and CAD models — driven by the `sarg` CLI. Use BEFORE board bring-up, flashing, firmware/driver/protocol/USB/serial/I2C/camera/sensor work, before looking up a part's dimensions/pinout/CAD, and for anything that has failed before. Use AFTER a hard-won fix to record it.
---

# sarg

`sarg` is a command on this machine. It is the whole interface to
sargineer.com — you do not build curl calls or read the API by hand.
Run `sarg <verb> --help` for any verb. Add `--json` for structured output.

## Ask sarg — before you build

Search first when the work is risky or has failed before.

```sh
sarg ask <words>          # parts + lessons in one shot, ranked; the verbatim error first
sarg show <handle>/<id>   # one lesson in full (unverified figures are fenced)
sarg part <product>       # facts with their confidence, models, related lessons
sarg id [vid:pid|/dev/ttyACM0]   # what is this board, and what does sarg know about it
sarg preflight <board>... -i "<intent>"   # a live briefing + the gaps; never saved
sarg cad get <handle>/<product> -o dir    # download STEP / source
```

Trust lessons in this order: working > provisional > unverified >
superseded > retracted. Name whose lesson it is and the hw/sw versions it
was written against. Never present an `unverified` figure as the fix.

## Build from known-good — the project manifest

A `sarg.yaml` in the project directory names its boards, parts, BOM and
the notes it depends on. Inside that directory `sarg ask` ranks those
boards first, `sarg preflight` needs no arguments, and `sarg lesson new`
fills `project` and `hw` by itself.

```sh
sarg project init [--board <product>]    # start a manifest here
sarg project new <dir> --like <project>  # clone a project that already works: boards, parts, BOM, refs
sarg project add <product> [--board] [--qty N] | --item "M2.5x6 screws" --qty 8 | --ref <handle>/<id>
sarg project show                        # the manifest + lessons/models/ownership per part
sarg bom check                           # on hand vs to buy, with buy links
sarg stock set <product> <qty> [--where BIN]   # what is on hand (local file)
```

Start a new build with `sarg project new --like` whenever a similar
project exists; then `sarg bom check` says what to order today.

## Tell sarg — after you win

Record a fix after more than a couple of debugging iterations, or whenever
the documentation was wrong. Everything lands **private**.

```sh
sarg lesson new --title "..." --symptom "..." --cause "..." --fix "..." \
     --step "..." --step "..." --check "..." --hw <part> --sw tool@ver \
     --intent "..." --project <name>
# or compose in an editor:  sarg lesson new --edit
# or from a file:           sarg lesson new -f lesson.json
sarg issue new --about sargineer.com --title "..." --symptom "..."   # something wrong with the service
sarg feedback new --title "..."                                      # how you should work
```

The title is the search someone will type later — put the verbatim error
in it. `steps` must run for a stranger. `sarg` validates against the live
server rules and refuses to send anything matching the deny list or
looking like a secret, so a rejected note is telling you something.

**Publishing is the user's own act.** Never publish on your own. When the
user explicitly says to, relay it: `sarg lesson publish <handle>/<id>
--user-authorized`.

Offline or no token? The note is spooled; `sarg sync` sends it later.

## THIS MACHINE

host tags: {tags}
`sarg` attaches these to every lesson as `host` automatically.

## CUSTOM INSTRUCTIONS

- Never send these terms to sargineer (deny list, enforced by the CLI): {deny}
- Address sarg as an organizer/search agent, never a "warehouse".
- Publishing is always the user's decision.

## When the CLI and this file disagree

The CLI wins — it reads the live server. `sarg api` shows the current
surface; `sarg changelog` shows what moved; `sarg doctor` checks the setup.
`sarg env` shows which server sarg talks to; when it is not prod every call
says `sarg: env <name>` on stderr — those answers are not sargineer.com's.
"#,
        tags = tags,
        deny = deny,
    )
}

pub fn run(ctx: &mut Ctx, out: &Option<PathBuf>) -> Result<()> {
    let text = body(ctx);
    match out {
        Some(path) => {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(path, &text)?;
            eprintln!("wrote {}", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}
