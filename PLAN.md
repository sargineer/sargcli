# sargcli — plan (rev 3, 2026-09-09; rev 2 was 2026-09-02)

`sargcli` is the Rust crate; the binary is `sarg`. It is the primary
interface to sargineer.com for humans and agents: **ask sarg** (search,
read, preflight, identify), **tell sarg** (lessons, issues, feedback,
ownership), and, later, **run the bench** (supplies on hand, project
manifests, BOMs). Evidence for every design choice came from a study
of a month of agent transcripts (kept out of the repo; it is the user's
private session history); the short version is in §1.

## 1. What the transcripts say (why rev 2 differs from rev 1)

- Claude asked sarg *before* hardware work in about 1 session in 3. The
  misses were not ignorance of the skill: it was installed and in context
  every time. Stale local knowledge (memory files, PREFLIGHT dumps, old
  `localhost:8093` pointers) satisfied the urge to look things up.
  → **The trigger must be mechanical (hooks), not prose.**
- Case design, dimensions, "find the cad model", "what do I own" never
  fired the skill at all. → **Trigger vocabulary covers parts and CAD,
  not just flashing.**
- One search = 3–4 endpoints; one design upload = ~22 calls; publish
  returns an empty 303; docs were refetched 60+ times in three weeks;
  five queries dumped 43 KB into context. → **One verb per intent,
  compact by default, docs local.**
- The raw token was inlined into shell commands dozens of times. An
  SSID, home paths and a client name each nearly left the machine.
  Placeholder titles were posted verbatim. → **The CLI is the trust boundary: never
  prints the token, scans and validates every outbound note.**
- The user asked for a project concept (08-23) and the only quantity
  data on supplies is a 2026-07 `queue.yaml` in `~/.sargineer/stock/`.
  → **Projects and stock are a real phase, designed now, built after
  the core.**

## 2. Principles

1. **Server is the authority.** Validation reads live `note_fields`
   from `/api` (cached 24 h); version drift is surfaced; `sarg raw`
   reaches anything without a verb. Every server issue we filed is
   tracked in §6 and adopted as it lands.
2. **One verb per intent.** `ask` fans out and merges. `cad put` does
   create + declare + PUTs. Mutations always print resulting state
   (the CLI re-GETs after a 303).
3. **Compact by default.** One line per hit; `-v` for a full lesson;
   `--json` or a pipe for machines. Context tokens are the scarcer budget.
4. **Trust boundary in code.** Token never printed. Every outbound note
   passes: live-schema validation, placeholder rejection, secret scan
   (SSIDs, tokens, emails, home paths, LAN IPs), deny-term list from
   config. Publishing requires a TTY confirm or `--user-authorized`.
5. **Status-honest.** Handle · status · versions · date on every hit;
   `unverified` fenced; a guessed cause is fenced automatically.
6. **Never lose a lesson.** No token or network → spool to
   `~/.sargineer/pending/`; `sarg sync` uploads.
7. **Measure ourselves.** `~/.sargineer/journal.jsonl` records each CLI
   call (session, verb, hits). `sarg stats` answers "how often did we
   ask before flashing" without re-mining transcripts.

## 3. Command surface

```
# ask sarg
sarg ask <query...> [--hw part] [-n N] [-v] [--json]
      /search + /notes?full=1 (+ /p for matched parts) in parallel → one ranked,
      compact list; names gaps ("no lessons about X"); -v expands one hit
sarg show <handle>/<id>            one note, every field, unverified fenced
sarg part <product>                facts (with confidence flags), models by role, lessons
sarg id <vid:pid | /dev/ttyACM0>   what is this device, and what does sarg know about it
sarg tag <t> | sarg vendor <v>
sarg preflight [<board>...] [--intent ...]   live, never persisted; boards default to
                                             the project manifest (§5)
sarg cad ls <handle>/<product>     files grouped by role: print / cad / source / image
sarg cad get ... [--role r] [-o dir]

# tell sarg
sarg lesson new [-f note.yaml|json] [--edit] [--capture-versions] [--fix-verified|--cause-verified]
sarg lesson ls [--mine] [--hw part] | edit <id> | rm <id>
sarg lesson publish|hide <handle>/<id>       guarded; prints resulting visibility
sarg issue new --about sargineer.com|<vendor> ...
sarg feedback new ...
sarg own -f invoice.txt | sarg own add <product>
sarg cad put <dir> [--publish]                part.yaml + files → create/declare/PUT in one go
sarg sync                                     upload ~/.sargineer/pending/*

# make it automatic
sarg hook pre-bash | prompt | stop   Claude Code hook entry points (stdin JSON → stdout JSON)
sarg hook install                    writes the three hooks into ~/.claude/settings.json
sarg doctor                          stale pointers (localhost:8093, mywarehouse, "warehouse"),
                                     token location, skill version vs server
sarg skill                           emits the ~25-line SKILL.md that delegates here
sarg stats                           searched-before-hardware rate, lessons saved, from the journal

# bench (phase 4)
sarg project init|show|add|bom        sarg.yaml manifest in the project dir
sarg stock ls|add|take|where|low      quantities, bins, consumables; joins on sargineer product ids
sarg bom check [<project>]            on hand vs to buy

# plumbing
sarg auth status | login-link [--open] | token mint
sarg apply | sarg init | sarg config get|set
sarg api | changelog [--since] | doc search|share|start | raw <METHOD> <path> [-d @f]
```

Config `~/.sargineer/config.toml` (0600): url, token, host_tags,
deny_terms, project_aliases, default_project. Resolution: `--token` flag →
config file. **Never the environment and never an agent's settings file**
(user, 2026-09-02): a token that arrives through Claude's env block is a
token that came from Claude's settings. Cache `~/.sargineer/cache/`
(api.json, later the tag dictionary for the prompt hook). Exit codes: 0 ok,
1 error, 2 usage, 3 auth, 4 validation/blocked, 5 spooled offline.

**Which agent is calling** is decided in one place, `src/agent.rs`:
`SARG_AGENT` override → Claude Code markers (verified: `CLAUDECODE=1`,
`CLAUDE_CODE_SESSION_ID`, `AI_AGENT=claude-code_<ver>_agent`) → other
agents' markers (unverified, flagged as such in code) → `human` on a TTY →
`unknown`. Every journal line carries `agent` and `session`; `auth status`
prints "called by …". Phase 2 code is organised as `hooks/core.rs`
(agent-neutral guard/hint/nudge, plain verbs) plus one adapter per agent
protocol (`hooks/claude.rs` first), selected by `sarg hook <agent> <event>`
and never by guessing.

## 4. Hooks — the fix for "why didn't you check sarg first"

All three are thin: read hook JSON on stdin, call the same library the
verbs use, answer in under a second from cache where possible.

- **PreToolUse (Bash)** `sarg hook pre-bash`: match the command against
  hardware patterns (esptool, mpremote, picotool, nrfutil, dfu-util,
  idf.py flash, arduino-cli upload, openmv, /dev/tty*, meshcore-cli).
  Extract board hints from the command and the manifest. If no relevant
  `ask` has run this session, run one (compact, n=5) and return
  `deny` with the hits in the reason. Second attempt passes. No hits →
  allow silently. Journal keyed by the hook's session id.
- **UserPromptSubmit** `sarg hook prompt`: match prompt words against the
  cached dictionary (owned parts, hw tags, model names, vendors). Inject
  one line of additionalContext: "sarg: 5 lessons · 2 models · you own it
  — `sarg ask openmv-n6`". Covers the case-design and dimension misses.
- **Stop** `sarg hook stop`: if the journal shows hardware commands and
  no `lesson new` this session, one line: "N hardware commands, no lesson
  saved — `sarg lesson new --edit`".

`sarg hook install` adds these to settings.json (via the update-config
skill's conventions); `sarg doctor` verifies they are present.

## 5. Projects and stock (phase 4 design, decided now)

**Split by nature.** Identity and knowledge are shared and live on
sargineer (product ids, facts, models, lessons, ownership via `/own`).
Quantities, bins, consumables and per-project allocation are personal
operational state and live locally, keyed by sargineer product ids so
they can move to the server if it grows such records.

- **`sarg.yaml` manifest** in a project directory: name, `project` slug
  (matches the lesson field and the homenotes `note proj` slug), boards
  and parts by product id, BOM lines (product id or free text, qty),
  related note/model ids, deny_terms overrides. Effects: `ask` and
  `preflight` scope to the manifest's boards when run inside the
  directory; `lesson new` fills `project` and `hw`; `bom check` reads it.
- **Stock** at `~/.sargineer/stock.yaml` (git-friendly, one item per
  product id or consumable slug): qty, unit, bin/location, min_qty,
  notes, source (invoice line). Seeded once from
  `~/.sargineer/stock/queue.yaml` + the Adafruit/Amazon exports, and
  from `/stack`. `sarg own` writes both places. `stock take` decrements
  when a BOM is built; `stock low` lists below-min.
- **Server asks to file** once this proves out: qty/location on `/own`;
  a `project` record (members: parts, notes, models; a BOM); consumables
  as a part kind (today `/own` skips "wire" lines).

## 6. Server issues we depend on (filed; re-check `sarg changelog`)

GET /find combined search · POST /parts/bundle · JSON body on publish ·
hardware-boosted ranking · `ruled_out` field · USB vid:pid lookup · pcb
kind + publish-board.md · per-dimension confidence · compact rendering.
The CLI papers over the first three and the last one today; it adopts
each server feature when the version moves.

## 7. Phases

**0 — skeleton. DONE 2026-09-02.** clap tree, config/auth resolution, HTTP
client (bearer, hint passthrough, redaction), `--json`, exit codes, `/api`
cache + drift notice, journal. Verbs: `auth status|login-link|token-mint`,
`api`, `changelog`, `doc`, `raw`, `config`, `init`. 11 mock-server tests,
clippy clean, built in claude-box (image now ships Rust; crates.io on the
allowlist). Verified live: `sarg init --from-claude-settings` → `signed in
as sargbench2`. Deviation from §2.3: output is compact text unless `--json`
is passed; no TTY auto-detection, since agents read through a pipe and text
costs fewer tokens than JSON.

**1 — ask sarg. DONE 2026-09-02.** `ask` (parallel /search + /notes?full=1,
merge, rank by status then hw match, compact/`-v`/`--json`, names the gap),
`show` (unverified fenced), `part` (estimate flags surfaced, models by
role, made-part links, bring-up digest), `tag`, `vendor`, `id` (sysfs →
vid:pid → chip table → lessons that mention it), `preflight` (live, never
persisted, gaps listed), `cad ls/get` (roles, 401 explained). 24 mock tests.
Live: `sarg ask heltec v4 meshcore` puts the flash recipe first in one
screen; `sarg id 303a:1001` names Espressif USB-JTAG and /dev/ttyACM0.
**Ownership is out of scope for now (user, 2026-09-02):** a `stack` verb and
an owned-parts cache were built and then removed; `id` no longer guesses
which owned board a device is. Phase 4 revisits ownership only if wanted.

**Agent neutrality (decided before phase 2).** Everything so far is
agent-agnostic: any agent that can run a shell command and read stdout can
use `sarg`, `--json` gives it structure, the token comes from env or
config. Only two things are Claude-specific: `init --from-claude-settings`
(a convenience) and the journal's session id, which reads
`CLAUDE_SESSION_ID` and falls back to `SARG_SESSION`. Phase 2 keeps it that
way: the guard logic lives in plain verbs (`sarg guard <command>` → exit
code + text, `sarg hint <prompt text>` → one line or nothing) and the
Claude Code hook protocol is a thin adapter (`sarg hook claude …`). Other
agents wire the same two verbs into their own hook systems (Codex,
opencode plugins, Cursor hooks, a shell wrapper around `esptool`); the
regenerated skill uses the cross-agent SKILL.md format sargineer already
serves. `sarg mcp` (phase 5) covers agents that prefer tools over shell.

**2 — make it automatic. DONE 2026-09-02.** Neutral core (`src/hooks/core.rs`):
`guard` (flash/bring-up commands must be preceded by a search — blocks once
with hits, allows after an `ask`/`preflight`/`id` or a prior block this
session), `hint` (offline keyword match → one `sarg ask` line), `nudge`
(board work + no lesson → reminder). Exposed as agent-neutral verbs
`sarg guard`/`sarg hint`, plus `sarg hook claude pretooluse|userpromptsubmit|stop`
(the only Claude-specific code, in `src/hooks/claude.rs`). `sarg hook
install|uninstall|status` edits `~/.claude/settings.json` (backs it up,
idempotent). `sarg doctor` (token, config, deny_terms, agent, server drift,
stale pointers incl. :8093/mywarehouse, hooks), `sarg stats` (journal:
calls, agents, guard blocks, notes written), `sarg skill` (emits a
delegating SKILL.md carrying host_tags + deny_terms). **Hooks are built and
tested but NOT auto-installed** — the user runs `sarg hook install` when
ready, since a PreToolUse hook affects every future Bash call.

**3 — tell sarg. DONE 2026-09-02.** Live-schema validator (`src/notes.rs`,
required per kind, status enum, list vs single-line, `<placeholder>` and
too-short/example titles rejected, body-length + missing-error warnings),
secret scan (`src/secrets.rs`: deny_terms and the token are hard stops;
home paths / emails / private IPs block unless `--allow-pii`), host tags +
project/alias auto-fill, `lesson new/ls/edit/rm`, `issue`, `feedback`,
`publish`/`hide` (the user's act: TTY confirm or `--user-authorized`, then
GET to echo state past the empty 303), TOML/JSON `-f` and `--edit`, offline
spool + `sync`. Done: an agent lesson is accepted first try; a note with a
deny term or PII is refused with exit 4 naming the field.

**4 — bench. Was SKIPPED 2026-09-02; REOPENED and DONE 2026-09-09 as step
A of the rev 3 roadmap (§9).** The framing changed: not inventory, but the
project as the unit of reuse. Built: `src/manifest.rs` (sarg.yaml, walk-up
discovery, slugify), `sarg project init|show|add|new --like|ls` (registry
in config `projects`, alias auto-set), `sarg bom check` (stock file → server
`owned_qty` → buy link), `sarg stock ls|set|rm` (`~/.sargineer/stock.yaml`).
Manifest-aware: `ask` boosts the project's boards and names the project,
`preflight` defaults to them, `lesson new` fills `project`/`hw`, the flash
guard searches them when the command names no hardware. 9 project tests +
3 unit tests; 70 total. Found and fixed on the way: the private-IP scanner
overflowed a u16 on any number > 65535 in a note ("256000 baud").

**5 — later, not done.** `sarg mcp` (verbs over MCP); `lesson draft
--from-log` via gpurouter; `eval claim/heartbeat/result`; `--capture-versions`;
`cad put`; shell completions; aarch64 build. None built.

## 8. Stack and hygiene

clap (derive) · ureq (rustls, blocking; hooks must start fast) ·
serde/serde_json/serde_yaml · toml · directories · anyhow · anstyle ·
dialoguer · open. Tests: validator against a checked-in `/api` fixture,
verbs against the std fixture server in `tests/common` (§8c; was
`httpmock` until 2026-09-21), hook entry points against recorded hook JSON.
No network in `cargo test`. First real build pulls crates → run in
`claude-box ~/code/sargcli` unless told otherwise.

## 8b. Environments (2026-09-09)

`sarg env` — `prod` is the top-level url/token; `[envs.<name>]` adds a dev
server with its own token, cache, spool and last-seen version
(`~/.sargineer/envs/<name>/`). Active env is `env = "<name>"` in config.toml
so hooks follow; `--env NAME` for one call; stderr banner on every non-prod
call; doctor flags it. Built for the local sargineer-web dev instance
(`sargineer-dev.service`).

## 8c. The language question, and what it actually costs (2026-09-21)

Asked directly: should this be another language? Answered: **no, it stays
Rust** — and the reasons are worth writing down, because the question will
be asked again by whoever next waits eight minutes for a release build.

Two things make a static binary the right artifact, and neither is about
Rust being fast at computing anything. The hook runs on *every* Bash
PreToolUse (`~/.claude/settings.json`), synchronously, in the agent loop:
measured cold, `sarg --version` is ~0.00s wall and ~3.9 MB RSS. A Python
or Node `sarg` pays 40-100 ms of interpreter start per tool call, hundreds
of times a session — that alone settles it. And the install line is the
product: `install.sh` / `install.ps1` put a working `sarg` on a machine
nobody here controls, and the lock resolves TLS to rustls+ring rather than
native-tls, so there is no system OpenSSL to have. A runtime dependency
would be re-introducing the problem the installer exists to remove.

Go is the only alternative that keeps both properties, and on a greenfield
day it would be close. It is not greenfield: v0.1.1 is released and
installed, Windows is validated in CI, and ~4.6k lines of tests encode
things somebody already lived through (the SIGPIPE hook in `main.rs` is a
bug, not a style).

What the code says about the fit: **0 `unsafe`, 4 lifetime annotations, 6
generic fns, 2 hits for Arc/Mutex/async, 114 `.clone()` calls.** This is
scripting-style Rust that clones past the borrow checker. Little of Rust's
distinctive power is being used — and little of its tax is being paid.
Nobody is fighting this compiler.

**The cost is the test harness, not the language.** Splitting the lockfile
closure: 238 packages, 146 in the runtime closure, and **92 (39%) exist
only for `dev-dependencies`**. `httpmock` drags in `async-std`, a whole
async executor stack, and `lalrpop` — a parser generator, compiled from
source on every clean build — all for a harness whose job is to serve
canned JSON on localhost and assert what the CLI prints.

*Measured, once it was done, because the first version of this paragraph
guessed and guessed high.* Clean `cargo clean && cargo test` in the box,
32 cores, cargo cache warm:

  with httpmock    203 crates · 8.4 s wall · 83 s CPU · target 1.5 GB
  without          108 crates · 6.6 s wall · 37 s CPU · target 750 MB

So the honest gain is half the CPU, half the disk, and a hundred fewer
packages to trust — not the minutes this section originally implied. Local
builds on this machine were never slow; 32 cores hide a lot. The 9.7 GB
`target/` was accumulated cruft across debug, release, the Windows cross
target and `distrib`, not one build's footprint, and the 4-8 minute CI runs
are release builds cross-compiling several targets and uploading, which
this change barely touches. Worth doing for the smaller trusted surface and
the halved CPU; not worth claiming it fixed a build-time problem.

*The work (DONE 2026-09-21, branch `test/drop-httpmock`):* `httpmock`
replaced by a fixture server over `std::net::TcpListener` in
`tests/common/mod.rs` — binds :0, accept-loop thread plus a thread per
connection, answers from a route table, joins on drop. Lock 250 -> 150
packages; 94 tests before and after, same outcomes. Supersedes the
`httpmock` line in §8.

Two traps found in the building, both worth knowing before anyone writes
another fixture server here: closing a connection with unread bytes in
flight gives the peer an RST, and ureq *silently retries* — which
intermittently doubles a mock's hit count while the CLI notices nothing.
And never closing, with one accept-loop thread, head-of-line blocks the
next test behind the connection `sarg_piped_into_head` deliberately
abandons. The read timeout is 60 s and must stay generous: it also bounds
the wait between two requests on one pooled connection, and
`edit_gates_pii_the_same_way_as_new` runs $EDITOR in that gap — time out
there and ureq will not retry the PUT that follows.

*Watch item:* `serde_yaml` is pinned at `0.9.34+deprecated` and upstream is
archived. Two files use it (`manifest.rs`, `stock.rs`). `serde_yml` or
`saphyr` is the move when it bites; not urgent, but it will not un-archive
itself.

## 9. Decisions taken (change if you disagree)

- Binary `sarg`; verbs read as asking sarg to do things.
- Hooks come before recording (phase 2 before 3): the trigger failures
  cost more than the recording friction, and hooks only need `ask`.
- Stock and project state are local files keyed by product id, not
  server records, until the server grows them.
- Flags/YAML-in-editor first; no TUI. `--json` everywhere.
- Token lives in `~/.sargineer/config.toml` (or `--token`), never the
  environment and never an agent's settings file.
- Rust, and it stays Rust: the PreToolUse hook is on the agent's
  critical path and the installer must not assume a runtime (§8c).

## 9. Roadmap rev 3 (2026-09-09) — build fast, reuse what is known to work

Reassessment after phases 0–3 ran for a week. The journal says sarg is a
guard: 240 PreToolUse checks, 2 flash commands held, ~30 real asks, 19
lessons in 15 sessions. It stops a repeated mistake; it does not yet hand a
builder the fastest known path. The corpus is failure-shaped (symptom →
cause → fix) and retrieval is by keyword. Four gaps, in build order. The
first two need no server change beyond `/own`, which exists. Scope guard:
sarg does not grow toward CAD or PCB authoring; models stay pointers with
confidence flags.

**A — the project as the unit of reuse (phase 4, reopened).**
`sarg.yaml` manifest per project dir: name, `project` slug, boards and
parts by product id, BOM lines, recipes/notes/models it depends on,
deny_terms overrides. Verbs: `sarg project init|show|add`,
`sarg project new --like <project>` (clone the boards, recipes, case and
starter code of one that already works), `sarg bom check` (on hand vs to
buy, vendor links from part facts). Effects: `ask`/`preflight` scope to the
manifest's boards; `lesson new` auto-fills `project` and `hw`. Stock stays
minimal: `~/.sargineer/stock.yaml` keyed by product id, written by `own`
and `bom check`, no bins/consumables UI unless asked.

**B — retrieval by need, owned first.** Capability tags on parts (mcu,
radio, sensor type, power input, interfaces) and `sarg find --need "lora
gps lipo"` ranking by what has a working recipe, with owned parts first.
Ownership comes back in its minimal form: `sarg own -f invoice | add
<product>` over `POST /own`, and an owned filter on `find`/`ask`. Not a
stock spreadsheet. Works-with edges generalise made-part `fits` to
board+sensor+library+case pairings; the filed `ruled_out` field is the
negative edge.

**C — success-shaped records with freshness.** A `recipe` (bring-up)
record: hw, sw@exact version, wiring, commands in order, the check, last
verified date. `preflight` already assembles this live; persist it. A
"still works" tap on lessons and recipes (the `POST /p/<product>/printed`
pattern) so "known to work" carries a date. A `starter` role/kind pointing
at a git ref + commit + verified date, so firmware skeletons (e.g. the
dustycam camera standard) are reusable artifacts, not just CAD. Needs
server asks: recipe kind or status, verify endpoint, starter role.

**D — capture, or the corpus never fills.** 30 Stop nudges → 19 lessons.
`lesson draft --from-log` (phase 5 item) drafts a lesson or recipe from the
session journal + shell history through gpurouter locally, user confirms.
A successful flash after a preflight is already a recipe in the journal.

**E — versions, captured at the moment of use.** Only 23% of notes name
a version (174 of 747). `sw` is already documented as `tag@version`
(`cli.rs:285`) and the skill already prints `--sw tool@ver`
(`skill.rs:85`) — nothing enforces it and nothing supplies it, so
`assemble()` splits on commas and writes whatever it was handed
(`record.rs:83-94`). A lesson that cannot say which version it was seen
on cannot answer "does this still apply", which is what freshness (C)
and the vendor board both rest on.

*Format.* `pkg@version`, split on the **last** `@`, so a scoped npm name
(`@tscircuit/cli@0.1.1974`) parses unchanged. Warn on an unparseable
version; never rewrite it.

*Capture at the event, not at the write.* Two things make write-time
probing wrong, both observed: `idf.py` is absent and `IDF_PATH` unset on
this workstation while `esp-idf` has 95 uses in the corpus — that work
happened on another machine; and `/dev/ttyACM0` is right now held by
another process, so a board probe at lesson time would fail or fight for
the port. The hook already knows the tool at the moment it runs —
`flash_tool()` returns it and `claude.rs:76-85` journals it as
`guard_tool`. Probe the version *there* and journal it beside the tool.
`journal::note()` already takes arbitrary keys, so this is a field, not
an architecture.

*The registry already exists.* `FLASH_TOOLS` (`core.rs:16-35`) is 18
tools; each gets a probe. Verified on this host: `esptool version` →
`esptool v5.3.1` then `5.3.1` (two lines — take the last), `mpremote
--version` → `mpremote 1.28.0`, `arduino-cli version` → `... Version:
1.5.1 Commit: ...` (regex), `kicad-cli version` → bare `10.0.6` (binary
is `kicad-cli`, package is `kicad` — the table carries the alias).
`esptool.py` is deprecated and prints a warning, so try `esptool` first.

*The mpremote trap, which the corpus already fell into.* `mpremote
--version` is the **host tool**, not the MicroPython on the board. The
corpus carries `micropython@1.28.0` 17 times, which is exactly mpremote's
version — almost certainly this conflation. So `mpremote` is its own
package, and `micropython@X` may only come from the device
(`sys.implementation`). Each probe declares its source and the CLI prints
it, so a wrong attribution is visible rather than silent.

*Serial safety.* Before any device probe: resolve through
`/dev/serial/by-id`, check for a holding process, and skip silently if
held. Never take a port someone else has.

*One new capability.* There is no run-a-command-and-capture helper today
— the only `std::process::Command` in the tree is `open_editor`
(`record.rs:194`), and `sarg id` reads `/sys/bus/usb/devices` directly
rather than shelling to `lsusb`. So this adds one: argv only (never a
shell string), binaries restricted to the probe table, ~2s timeout,
failure is "unknown", never an error.

*Surfaces.*
  - `sarg versions` — read-only, prints `pkg@ver` with the source of
    each; `--json`; `--sw` to emit paste-ready flags.
  - `sarg lesson new` — pre-fills `sw` from this session's journal,
    printing each with its source so the agent can drop the irrelevant
    ones. `--no-probe` to skip, matching `sarg init`.
  - `sarg doctor` — shows the detected toolchain. It already accumulates
    ok/problem lines and already runs where shelling out is acceptable.
  - `sarg.yaml` — an optional `toolchain:` block, the versions a project
    is known to build with. Gives `lesson new` a default and makes drift
    checkable the way `bom check` checks parts.
  - `sarg skill` — one rule, stated plainly: write `sw` as `pkg@version`,
    take it from `sarg versions` or let `lesson new` fill it, and
    **never invent a version number**. A fabricated version makes a
    lesson look verified when it is not, which is worse than a bare name.

Server side is small and already specified in sargineer's MATCHING.md §7:
`sw` needs no new note field — it *is* the observed environment. Parsing
into `note_sw` with a sortable key, and `fixed_in`, come with the vendor
responder flow, not here.

Order: A, then B, then C, then D, with E alongside D. A and B are CLI-only. **A DONE 2026-09-09**
(see phase 4 above). Noted while smoke-testing A live: `openmv-n6` is a hw
tag with 53 lessons but not a product id (`/p/openmv-n6` → 404), so
`project show`/`bom check` cannot annotate it — B's capability tags should
make tag-vs-product a non-issue, or the server should alias them.
