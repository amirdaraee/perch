# Perch — a dashboard for Claude Code

**Date:** 2026-08-30
**Status:** Design approved, ready for implementation planning

---

## 1. Problem

Many Claude Code sessions run at once across many projects. Three things are hard:

1. **Triage** — which session is working, which is blocked waiting on input, and which terminal owns it.
2. **Continuity** — what each project was doing and where it left off.
3. **Budget** — how much of the rate-limit window is consumed, when it resets, and where the tokens went.

Claude Code writes all the raw material to disk but offers no cross-session view.
Perch reads that data and presents it.

A concrete instance of the problem, found while designing this: a background session
in `personal-dashboard` had been blocked on `waitingFor: "dialog open"` for 32 hours,
invisible.

## 2. Scope

**In scope**

- Auto-discovery of projects and sessions from Claude Code's own data, with a light
  curation layer (rename, pin, status, one-line note).
- Live session monitoring with status, including notifications for blocked sessions.
- Token and estimated-cost accounting per session, project, model, and day.
- Rate-limit window/weekly utilization and reset times, with a local-estimate fallback.
- Actions: jump to the owning terminal, resume an ended session, open project paths.
- macOS first; structured so Windows/Linux are additive.

**Out of scope**

- Sending messages to running sessions over the session socket. Deferred; the socket
  protocol is undocumented and the value is lower than the risk.
- Editing or deleting Claude Code data. Perch is read-only against `~/.claude`.
- Any non-Claude-Code AI tool.
- Multi-machine sync or a hosted component.

## 3. Data sources

All paths resolve as: `$CLAUDE_CONFIG_DIR` → `$XDG_CONFIG_HOME/claude` → `~/.claude`.
A settings override exists, and a clear empty state appears when no data directory is found.

| Source | Path | Provides |
|---|---|---|
| Live session records | `<config>/sessions/<pid>.json` | `pid`, `sessionId`, `cwd`, `name`, `kind` (`interactive`/`bg`), `status` (`busy`/`waiting`), `waitingFor`, `statusUpdatedAt`, `startedAt`, `version` |
| Session sockets | `/tmp/cc-socks/<pid>.sock` | Liveness corroboration |
| Transcripts | `<config>/projects/<slug>/<uuid>.jsonl` | Per-message `model` and `usage` (input, output, cache read, cache write 5m/1h, thinking), `cwd`, `gitBranch`, `version`, timestamps |
| Prompt history | `<config>/history.jsonl` | Typed prompts keyed by `sessionId` and project — session titles and prompt search without parsing transcripts |
| Daily rollups | `<config>/stats-cache.json` | Message/session/tool counts per day (cross-check only) |
| Usage endpoint | `/api/oauth/usage` on Anthropic, OAuth token from the macOS Keychain item `Claude Code-credentials` | Window and weekly utilization, reset timestamps |

### 3.1 Path resolution — do not decode slugs

Project directory names encode the path by replacing `/` **and** `.` with `-`, which is
lossy and ambiguous. `-Users-amirdaraee-00-projects-amirdaraee-github-io` is a real
observed case with three plausible decodings; the true path is
`/Users/amirdaraee/00/projects/amirdaraee.github.io`, and a naive decoder picks a
sibling directory that also exists.

**Rule:** the slug is a directory key only. The real path comes from the `cwd` field
present on every transcript line — read the first line of any session file in the folder.
Slug decoding survives only as a last-resort display label for a folder containing no
readable sessions, and is marked in the UI as a guess.

Worktrees follow the same rule. A `cwd` containing `/.claude/worktrees/` yields the
parent project by truncating at that marker — no string-guessing against slugs.

### 3.2 Data volume

Measured on the reference machine: 231 transcripts, 120 MB, largest file 24.8 MB in
665 lines (~38 KB per line — pasted content, tool output, images). Full re-parse per
launch is wasteful and grows without bound, which motivates the incremental indexer.

## 4. Architecture

A single Tauri application, tray-resident: closing the window does not quit it, because
the app is also the background monitor. Chosen over Electron (size, always-running cost)
and native Swift (no path to Windows/Linux).

Rust owns four modules:

- **`discovery`** — enumerates project directories, resolves real paths via `cwd`,
  groups worktrees under parents.
- **`indexer`** — incremental JSONL reader (§5).
- **`live`** — file-watches session records, corroborates liveness, emits status events.
- **`limits`** — Keychain read, usage endpoint client, local-estimate fallback.

The frontend never touches JSONL. It queries the SQLite index through Tauri commands and
receives push events for live changes.

### 4.1 Platform boundaries

Three things are genuinely OS-bound and sit behind Rust traits from day one, so the
Windows/Linux port is implementing traits rather than untangling `#[cfg]`:

- `CredentialStore` — Keychain today.
- `ProcessProbe` — liveness and command-line inspection.
- `AppLauncher` — focusing a terminal window, launching `claude --resume`, opening paths.

## 5. Indexing strategy

Transcripts are append-only, so `(file_size, indexed_offset)` is both a change detector
and a resume point. Each scan seeks to the stored offset and parses only new bytes; a live
session costs microseconds per refresh instead of re-reading megabytes. Cold start is a
one-time full parse.

**Partial-line safety.** A session being written while we read it will have an incomplete
final line. The indexer parses up to the last complete newline, advances the offset to
exactly that point, and never past it.

**Malformed lines** are skipped and counted, never fatal.

## 6. Data model

SQLite at the platform app-data directory (`~/Library/Application Support/Perch/index.db`).

```
projects(id, slug, real_path, parent_project_id, display_name,
         status, pinned, note, note_updated_at, archived)

sessions(id TEXT PK, project_id, file_path, file_size, indexed_offset,
         started_at, last_activity_at, cwd, git_branch, cc_version,
         first_prompt, message_count, is_live)

turns(session_id, ts, model,
      input, output, cache_read, cache_write_5m, cache_write_1h, thinking_tokens)

usage_snapshots(fetched_at, window_pct, window_resets_at,
                weekly_pct, weekly_resets_at, source)   -- source: 'api' | 'estimated'

prices(model, input_per_mtok, output_per_mtok,
       cache_read_per_mtok, cache_write_per_mtok)
```

**Derived vs. owned.** `display_name`, `status`, `pinned`, `note`, `note_updated_at`,
`archived` are user-owned; everything else is re-derivable. Re-indexing **upserts on
`slug`** and never deletes from `projects`. A "rebuild index" command drops `sessions`
and `turns` only, so corruption is recoverable without losing user input.

**Token classes stay separate.** An observed single turn recorded 2 input / 23,848 cache
write / 24,221 cache read tokens — summing only input and output under-reports volume by
~99% and prices it wrongly. All four classes are stored and priced independently.

**Two clocks.** Rate limits reset on a rolling 5-hour window unrelated to calendar days.
`turns.ts` is stored raw so window queries and daily queries are both derivable.

## 7. Live session detection

Watch `<config>/sessions/` with `notify`. Each record supplies status directly — no
inference from file mtimes, which cannot distinguish "thinking for 3 minutes" from
"blocked on a permission dialog."

**Liveness requires three confirmations**, because a crashed session leaves its record
behind and pids are recycled:

1. The record file exists.
2. `kill(pid, 0)` succeeds.
3. The process's **executable name** — not its arguments — is `claude`.

The socket at `messagingSocketPath` is a supporting fourth signal. Failing the checks marks
the session ended and moves it to history.

> **Corrected 2026-08-31 against real data.** Confirmation 3 originally read "the process
> command line contains the matching `--session-id`". That is false for **interactive**
> sessions: on Claude Code 2.1.251 they show only `claude --dangerously-skip-permissions`,
> with no session id anywhere on argv. Only daemon-spawned `kind: bg` sessions carry
> `--session-id`. The original rule filtered out every interactive session — that is, almost
> everything the user cares about.
>
> Matching on the session id is also unnecessary, because **records are keyed by pid**
> (`<pid>.json`). A new session landing on a recycled pid overwrites the stale record rather
> than coexisting with it, so two records can never claim one pid.
>
> Read the executable name (`ps -o comm=`), never the full command line (`ps -o command=`).
> A substring search for "claude" over arguments would match any unrelated process holding a
> path like `claude-dashboard` — `vim ~/projects/claude-dashboard/x.rs`, for instance.
>
> **This narrows the residual hazard; it does not eliminate it.** One window remains: session A
> crashes leaving a stale record, the OS reuses its pid for a genuinely new `claude` process B,
> and B has not yet written its own record. In that interval every check passes — the pid is
> alive and *is* a Claude process — so A's stale record is briefly shown with A's name and cwd
> against B's pid. It self-corrects as soon as B writes its record (which overwrites A's). If
> that window ever proves observable, `procStart` is the fix: the record carries it, and
> comparing it against the process's real start time closes the race. It is deferred only
> because it needs timezone normalisation (the record's `procStart` is offset from
> `ps -o lstart=` by the local UTC offset).
>
> A stronger check remains available if ever needed: the record carries `procStart`, which can
> be compared against the process's actual start time. It is deferred because it requires
> timezone normalisation (the record's `procStart` is offset from `ps -o lstart=` by the local
> UTC offset) for no benefit the pid-keying argument does not already provide.

**Status model**

| State | Source | Presentation |
|---|---|---|
| Working | `status: busy` | green |
| Idle | `status: idle` | dim green — alive but not currently doing anything |
| Waiting on you | `status: waiting` | amber, with `waitingFor` reason and elapsed time from `statusUpdatedAt` |
| Background | `kind: bg` | grouped separately so long jobs do not nag |
| Ended | liveness failed | grey, resumable |

> **`idle` added 2026-08-31 against real data.** The spec originally listed only `busy` and
> `waiting`; real records also carry `status: "idle"`. Mapping it to Working would paint an
> idle session green as though it were mid-task. Any *further* unrecognised status still falls
> back to Working rather than inventing a state.

## 8. Usage and limits

**Tier 1 — measured.** Read the OAuth token from the Keychain, call `/api/oauth/usage`,
store the result with `source='api'`. Poll at most once per 60 seconds, enforced in code
rather than configuration, with exponential backoff on failure and an identifying
User-Agent. The Keychain read happens lazily on first open of the usage panel — never at
launch — preceded by an in-app explanation so the macOS prompt is not mysterious.

**Lenient parsing.** The endpoint is internal and undocumented. Responses deserialize with
defaults on every field and unknown fields ignored, so an upstream addition cannot break
the panel.

**Tier 2 — estimated.** On any failure (no token, 401, shape change, offline, or user
declined the Keychain prompt), fall back to summing `turns` over the trailing 5 hours and
trailing week. Absolute tokens and estimated cost only: the plan ceiling is unknown, so no
percentage is fabricated. Rendered as bars rather than gauges, with a dashed `est` badge —
a different visual treatment, not merely different wording.

The last successful tier-1 snapshot continues to display with an "as of HH:MM" stamp
rather than being replaced by an error state.

**Cost.** Tokens are truth; dollars are a labeled estimate from the editable `prices`
table, so a price change is a row update rather than a release. Unknown models are counted
with zero price rather than dropped.

## 9. User interface

### 9.1 Menu bar

The menu-bar item displays the **window utilization percentage**.

The popover is the **dense** layout: window / week / today as three side-by-side stats
with the reset countdown, followed by session rows carrying status, elapsed time, and
per-session token counts, including recently-ended sessions with a resume affordance.

### 9.2 Main window — hybrid layout

A left sidebar whose first entry is **Now**, with the project list beneath it
(pinned / active / recent / archived). Two top-level tabs: Overview and Usage.

- **Now** — a banner for any session waiting on input, the usage stat tiles, the live
  session table, and recently-ended sessions.
- **Project detail** — the project's note, aggregate tokens and spend, a 14-day sparkline,
  and its session history with per-session status and actions.

Rationale: triage is a many-times-daily glance and gets the permanent top slot; project
review is roughly daily and sits one click away. Neither is demoted behind a tab.

### 9.3 Usage view

Four blocks, all approved:

1. **Hero stats** — window %, week %, today's tokens and estimated spend, and a burn-rate
   projection ("at this pace the window fills in ~1h 50m"). The only predictive figure in
   the app.
2. **Daily stacked bars** — tokens per day by class (input, output, cache read, cache
   write), with hover detail.
3. **Top projects** — 30-day ranking, worktrees folded into parents, tail collapsed into
   "N others".
4. **By model** — tokens and estimated spend per model.

**Visualization rules.** Categorical hues assigned in fixed order from a validated
palette (dark-surface steps `#3987e5`, `#d95926`, `#199e70`, `#c98500`; all six checks
pass — lightness band, chroma floor, CVD separation, normal-vision floor, contrast).
Legend always present with direct labels; no dual-axis charts — dollars ride as secondary
labels rather than a second y-scale; tabular numerals throughout; recessive grid lines;
2px gaps between stacked segments.

### 9.4 Actions

- **Jump** — resolve the owning terminal via the process tree and focus that window;
  fall back to revealing the `cwd` in Finder.
- **Resume** — launch `claude --resume <sessionId>` in the configured terminal, defaulting
  to whichever application owns most current sessions.
- **Open** — project path in editor, Finder, or terminal.

### 9.5 Notifications

Off by default, individually toggleable.

- **Session waiting** — fires when a session has been in `waiting` longer than N minutes
  (default 10). Edge-triggered on the transition, not level-triggered on the state, so it
  fires once and re-arms only after the session returns to `busy`. `kind: bg` excluded by
  default.
- **Window at 80%** — once per window.

## 10. Open-source posture

The repository is public from the start.

**Privacy is structural.** Transcripts contain source code, credentials people pasted, and
client names. Perch ships with no telemetry, no analytics, and no crash reporting, and
makes outbound requests to exactly one host — `*.anthropic.com`, for the usage endpoint.
CI enforces this with a dependency and source audit; any new outbound host is a
review-blocking change. This claim is the reason a stranger will point the app at their
`~/.claude`.

**Demo mode.** A `--demo` flag backed by synthetic fixtures — invented projects and
plausible token curves. Required before any screenshot can be taken, since real screenshots
leak project and client names. It also serves the frontend dev loop and deterministic
integration tests.

**Endpoint honesty.** The README states plainly that `/api/oauth/usage` is an internal
endpoint, that it may change or break, and that Perch degrades to local estimates when it
does. Rate limiting and backoff are enforced in code. A "not affiliated with or endorsed
by Anthropic" notice is included.

**Distribution.** No Apple Developer account (the program is $99/year; a free Apple ID
cannot notarize for distribution). Releases ship unsigned with two documented paths:
`xattr -dr com.apple.quarantine /Applications/Perch.app`, or building from source, which
avoids quarantine entirely. Revisit notarization if adoption justifies the cost.

**Repo.** MIT license. GitHub Actions building macOS arm64 and x86_64 on tag.
`.superpowers/` git-ignored.

## 11. Testing

Test-driven throughout; the trait boundaries in §4.1 provide the seams.

| Area | Tests |
|---|---|
| Indexer | **Incremental equivalence** — parse fixture, append, re-parse from offset, assert totals equal a from-scratch parse. This property protects the entire performance strategy. Plus partial-final-line handling and malformed-line tolerance. |
| Path resolution | Table-driven: dots in names, dashes, worktrees, missing `cwd`, `CLAUDE_CONFIG_DIR` set and unset, ambiguous-slug regression case |
| Liveness | Faked `ProcessProbe`: stale record, dead pid, recycled pid with mismatched session id |
| Usage client | Recorded fixtures: success, 401, malformed body, unknown fields, offline — each asserting degradation to tier 2 rather than an error state |
| Cost | Known tokens × known prices; unknown-model path counted at zero price |
| UI | Light component tests; the demo fixture set carries the load by making every state reachable |

## 12. Build order

| # | Milestone | Done when |
|---|---|---|
| 1 | Discovery + indexer + SQLite | A debug command prints all projects with correct real paths and token totals |
| 2 | Live detection + tray + dense popover | The popover shows current sessions with correct statuses |
| 3 | Main window: Now + project list; jump and resume | Clicking a session row focuses the owning terminal |
| 4 | Usage view + limits client + fallback | Real percentages when authorized; labeled estimates when not |
| 5 | Notifications + curation (pins, notes, status) | A session blocked for 32 hours would have alerted |
| 6 | Demo mode, CI, README, unsigned release | A stranger can download and run it |

Milestone 1 is deliberately headless. If the data layer is wrong no UI saves it, and a
printing command iterates faster than a window.

## 13. Decisions and rationale

| Decision | Rationale |
|---|---|
| Tauri | Honours "cross-platform later" without paying for it now; small always-running footprint |
| SQLite + byte-offset incremental indexing | 120 MB and growing; append-only files make offsets a valid resume point |
| Read published `status`, never infer from mtime | mtime cannot distinguish thinking from blocked |
| Real path from `cwd`, never from the slug | Slug encoding is lossy and demonstrably ambiguous |
| Tier 1 API with tier 2 fallback | Real numbers when available; never a blank panel when upstream changes |
| Read-only, plus jump and resume | High-value, low-risk actions; socket messaging deferred |
| Unsigned distribution initially | $99/year is not justified before adoption |

## 14. Deferred

- Sending messages to live sessions over `messagingSocketPath`.
- Windows and Linux implementations of the three platform traits.
- Notarized, signed releases and a Homebrew cask.
- Prompt full-text search across `history.jsonl`.
