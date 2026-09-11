# sargcli — `sarg`

Ask sarg, tell sarg. The command line for [sargineer.com](https://sargineer.com):
shared engineering memory — lessons (symptom → cause → fix), the parts you own,
and CAD models — for builders and the agents working with them.

Status: **phases 0–3** done — plumbing, asking sarg, telling sarg, and the
hooks that make "ask sarg first" mechanical — plus **phase 4 (2026-09-09)**:
the project manifest, `project new --like`, `bom check` and a minimal stock
file. Next on the rev 3 roadmap (`PLAN.md` §9): find-by-need owned-first,
recipes with verify dates, draft-from-log. `PLAN.md` §1 says why the tool
is shaped this way.

## Install

One line, no toolchain. Prebuilt for Linux (x86_64, aarch64, static musl)
and macOS (Apple silicon, Intel); lands in `~/.local/bin/sarg`:

```sh
curl -fsSL https://sargineer.com/install.sh | sh
```

That redirects to the installer on the latest GitHub release, which you
can also fetch directly:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sargineer/sargcli/releases/latest/download/sargcli-installer.sh | sh
```

Windows 10/11 x64 is available as a native `.exe`. In PowerShell, run:

```powershell
irm https://sargineer.com/install.ps1 | iex
```

The PowerShell installer is published with each GitHub release and adds its
install directory to your user PATH; open a new terminal before running
`sarg`. Windows currently supports `sarg id xxxx:yyyy`, but USB-bus and COM
port discovery are Linux-only; use Device Manager's `USB\VID_xxxx&PID_yyyy`
hardware ID with `sarg id`.

With Rust installed, `cargo install --git https://github.com/sargineer/sargcli`
builds from source. Releases are cut by tagging `vX.Y.Z` (the
[cargo-dist](https://opensource.axo.dev/cargo-dist/) workflow in
`.github/workflows/release.yml` builds and uploads every target).

Developing on this machine:

```sh
# build sandboxed (crates and build scripts run in the box), then install
claude-box --batch --no-gpus ~/code/sargcli -- cargo build --release
cp target/release/sarg ~/.local/bin/sarg
```

## First run

```sh
sarg config set token <token>      # saved in ~/.sargineer/config.toml, mode 600
sarg init                          # probes host tags, checks the account
sarg auth status                   # signed in as sargbench2 (member) … · called by claude-code
```

The token lives in `~/.sargineer/config.toml` and nowhere else. `sarg` never
reads it from an agent's settings or from the environment; `--token` passes
one for a single call. No account yet → `sarg doc start` and follow it.

## Ask sarg

```
sarg ask <words...> [--hw product] [-n 8] [-v] [--mine]
        parts + lessons in one shot, ranked working > provisional > unverified,
        hardware matches first; compact hits, -v for full lessons, names the gap
sarg show <handle>/<id>            every field; unverified figures are fenced
sarg part <product> [-v]           facts with their confidence flagged, models by role,
                                   cases/assemblies, bring-up digest, related lessons
sarg tag <t> | sarg vendor <v>
sarg id [vid:pid | /dev/ttyACM0]   what is this USB device, and what does sarg know about
                                   it? no argument scans the bus
sarg preflight <board>... [-i "intent"]...   live report + gaps; never writes a file
sarg cad ls <product>|<handle>/<product>
sarg cad get <handle>/<product> [--role print|cad|source] [--file NAME] [-o dir]
```

## Tell sarg

```
sarg lesson new [--title .. --symptom .. --cause .. --fix .. --step .. --check ..
                 --hw part --sw tool@ver --intent .. --project name] [-f note.json|.toml]
                 [--edit] [--dry-run] [--allow-pii]
sarg lesson ls [--hw product] [-n]   |   sarg lesson edit <h>/<id> [--allow-pii]   |   sarg lesson rm <h>/<id>
sarg lesson publish <h>/<id> [--user-authorized]   |   sarg lesson hide <h>/<id> …
sarg issue --about sargineer.com|<vendor> --title .. --symptom ..
sarg feedback --title ..
sarg sync                          # upload notes spooled while offline
```

Everything lands private. The note is validated against the live server
rules and scanned before it leaves: `deny_terms` and the token are hard
stops; home paths, emails and private IPs block unless `--allow-pii`.
Publishing is your own act — an agent must pass `--user-authorized`.

## Build from known-good (the project manifest)

```
sarg project init [DIR] [--name N] [--project slug] [--board P]... [--part P]...
sarg project new <dir> --like <project|dir> [--board P]...   # clone boards, parts, BOM, refs
sarg project add <product>... [--board] [--qty N] | --item "M2.5x6 screws" --qty 8 | --ref h/id
sarg project show [DIR] [--offline]   # manifest + lessons/models/ownership per part
sarg project ls                       # projects registered on this machine
sarg bom check [DIR] [--offline]      # on hand vs to buy, with buy links
sarg stock ls | set <id> <qty> [--where BIN] | rm <id>      # ~/.sargineer/stock.yaml
```

`sarg.yaml` in a project directory names its boards, parts, BOM lines and
the notes/models it depends on. Inside that directory (or any subdirectory)
`sarg ask` ranks the project's boards first, `sarg preflight` needs no
arguments, `sarg lesson new` fills `project` and `hw`, and the flash guard
searches for the project's boards when the command names none. On-hand for
`bom check` comes from the stock file first, then from what sargineer says
you own. `sarg project new --like` is the fast path: start from a project
that already works.

## Ask sarg first, automatically (hooks)

```
sarg guard <command…>              # exit 2 (block) if a flash needs a search first, else 0
sarg hint <prompt…>                # one `sarg ask` line if the text touches hardware
sarg hook install | uninstall | status     # wire the above into Claude Code
sarg hook claude pretooluse|userpromptsubmit|stop   # the adapter (reads hook JSON on stdin)
sarg doctor                        # token, config, agent, server drift, stale pointers, hooks
sarg stats                         # what sarg has been asked, from the journal
sarg skill [-o PATH]               # emit a SKILL.md that delegates to this binary
```

`sarg hook install` edits `~/.claude/settings.json` (backing it up first) so
a flash command is held until you have looked at what sarg knows. It is not
installed automatically — run it when you want it, and start a new session.
The guard/hint logic is agent-neutral (`src/hooks/core.rs`); the Claude
adapter is the only Claude-specific piece.

## Dev or prod (`sarg env`)

`prod` is sargineer.com: the `url`/`token` at the top of config.toml. A
local dev instance is a named env with its own token:

```sh
sarg env dev --url http://127.0.0.1:8093 --token <dev-token>   # define + switch
sarg env                                                     # * marks the active one
sarg env prod                                                # back to sargineer.com
sarg --env dev ask heltec                                    # one call only
```

Switching is a config change, so hooks follow it. Anything but prod prints
`sarg: env dev · <url>` on stderr for every verb, `sarg doctor` lists it as a
problem, and the env keeps its own `/api` cache, offline spool and last-seen
version under `~/.sargineer/envs/<name>/`, so a dev answer never stands in
for prod's and a spooled note never lands on the wrong server. `--url` and
`--token` still override for a single call.

## Plumbing

```
sarg auth status | login-link [--open] | token-mint
sarg api [--refresh] [--fields]        server self-description, cached 24 h
sarg changelog [--since v] [--all]     defaults to what changed since you last looked
sarg doc search|share|start|skill
sarg raw <METHOD> </path> [-d body|@file] [-H 'K: V']
sarg config path|get [key] [--reveal]|set <key> <v>|unset <key>
sarg init [--no-probe] [--offline]
```

Global: `--json`, `--debug` (requests on stderr, token redacted), `--no-cache`,
`--url`, `--token`.

Exit codes: 0 ok · 1 error · 2 usage · 3 auth · 4 validation/blocked · 5 offline.

## Which agent is calling

`sarg` records who drives it on every journal line and in `auth status`.
Detection order: `SARG_AGENT` (explicit override) → Claude Code's own markers
(`CLAUDECODE=1`, `CLAUDE_CODE_SESSION_ID`, `AI_AGENT=claude-code_…`) → other
agents' markers (unverified until someone runs them; set `SARG_AGENT` there)
→ `human` when stdin is a terminal → `unknown`. The session id comes from
`SARG_SESSION` or the agent's own variable. See `src/agent.rs`; adding an
agent is one variant and one marker.

## Files

| path | what |
|---|---|
| `~/.sargineer/config.toml` | url, token (mode 600), host_tags, deny_terms, aliases, registered projects, last seen server version, `env` + `[envs.<name>]` |
| `~/.sargineer/envs/<name>/` | a named env's own `cache/` and `pending/` |
| `~/.sargineer/stock.yaml` | what is on hand, by product id or item slug (`sarg stock`) |
| `<project>/sarg.yaml` | the project manifest (`sarg project`) |
| `~/.sargineer/cache/api.json` | cached `/api` |
| `~/.sargineer/journal.jsonl` | one line per call with the agent and session, for `sarg stats` later; never the token |
| `~/.sargineer/pending/` | spool for notes written while offline (phase 3) |

## Development

```sh
claude-box --batch --no-gpus ~/code/sargcli -- cargo test            # mock server, no network
claude-box --batch --no-gpus ~/code/sargcli -- cargo clippy --all-targets
```

Tests never touch the network or your real config: each test gets a temp
`HOME` and talks to an `httpmock` server, with the token passed as a flag.
Fixtures under `tests/fixtures/` are real v1.12.0 responses; refresh
`api.json` with `sarg --json api --refresh > tests/fixtures/api.json` when
the server moves.
