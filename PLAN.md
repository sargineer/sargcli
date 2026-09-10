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
verbs against `httpmock`, hook entry points against recorded hook JSON.
No network in `cargo test`. First real build pulls crates → run in
`claude-box ~/code/sargcli` unless told otherwise.

## 8b. Environments (2026-09-09)

`sarg env` — `prod` is the top-level url/token; `[envs.<name>]` adds a dev
server with its own token, cache, spool and last-seen version
(`~/.sargineer/envs/<name>/`). Active env is `env = "<name>"` in config.toml
so hooks follow; `--env NAME` for one call; stderr banner on every non-prod
call; doctor flags it. Built for the local sargineer-web dev instance
(`sargineer-dev.service`).

## 9. Decisions taken (change if you disagree)

- Binary `sarg`; verbs read as asking sarg to do things.
- Hooks come before recording (phase 2 before 3): the trigger failures
  cost more than the recording friction, and hooks only need `ask`.
- Stock and project state are local files keyed by product id, not
  server records, until the server grows them.
- Flags/YAML-in-editor first; no TUI. `--json` everywhere.
- Token lives in `~/.sargineer/config.toml` (or `--token`), never the
  environment and never an agent's settings file.

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

Order: A, then B, then C, then D. A and B are CLI-only. **A DONE 2026-09-09**
(see phase 4 above). Noted while smoke-testing A live: `openmv-n6` is a hw
tag with 53 lessons but not a product id (`/p/openmv-n6` → 404), so
`project show`/`bom check` cannot annotate it — B's capability tags should
make tag-vs-product a non-issue, or the server should alias them.
