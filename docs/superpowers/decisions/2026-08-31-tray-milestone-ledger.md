# SDD ledger — plan: docs/superpowers/plans/2026-08-31-perch-tray-and-live-sessions.md

Spec: docs/superpowers/specs/2026-08-30-perch-design.md
Branch: feat/tray-and-live-sessions
Scope: spec Milestone 2 only.

## Pre-flight conflict scan

### Task pairs sharing a file
| Tasks | File | Finding |
|---|---|---|
| 1 → 2 | perch-core/src/live.rs | Clean (T2 appends live_sessions + tests) |
| 1,2 | perch-core/src/lib.rs | Clean (one `pub mod` line each, sequential) |
| 2 → 4 | root Cargo.toml | Clean (T2 adds libc to workspace deps; T4 adds src-tauri member) |
| 4 → 5 → 6 → 7 → 9 | src-tauri/src/lib.rs | Clean (create, replace, add invoke_handler, add watcher::spawn, edit) |
| 7 → 9 | src-tauri/src/watcher.rs | Clean |
| (m1) → 9 | .github/workflows/ci.yml | Clean (T9 adds a job, keeps existing two) |

### Task pairs sharing an interface
| Tasks | Produces → consumes | Finding |
|---|---|---|
| 1 → 2,3,6,7 | LiveSession, SessionStatus, parse_session_record, record_files | **F1** |
| 2 → 3,6,7 | ProcessProbe, RealProcessProbe, live_sessions | Clean |
| 6 → 8 | command names + UsageSummary/UsageSlice shape vs types.ts | Clean (camelCase rename matches) |
| 7 → 8 | event "sessions-changed" carrying Vec<LiveSession> | Clean |
| 5 → 9 | tray id "perch-tray" used by tray_by_id | Clean |
| 6 → core | IndexStats must derive Serialize (it does not today) | Already flagged inline in T6 |

### Per-task internal consistency
| Task | Finding |
|---|---|
| 1 | **F1** — serde attribute would not produce the field name the frontend expects |
| 2 | **F2** — libc shown as if it were a line inside [dependencies]; it is its own target table |
| 3,4 | Clean |
| 5 | **F3** — `default_window_icon().unwrap()` panics on launch if absent |
| 6,7,8,9 | Clean |

### Rulings (all applied to the plan before Task 1 dispatch)

Ruling: F1 — `#[serde(tag = "kind", rename_all = "camelCase")]` renames enum VARIANTS only, not the
  fields inside struct variants. `since_ms` would have serialized as "since_ms" while `types.ts`
  expects `sinceMs`, so the popover's elapsed timer would read `undefined` and render NaN — a
  silent UI failure with no compile or test error, since the two sides are in different languages.
  Added `rename_all_fields = "camelCase"`. — Cost if wrong: caught immediately by the Task 8
  hand-verification, but only after the UI is built on a broken contract.

Ruling: F2 — clarified that the libc dependency is a `[target.'cfg(unix)'.dependencies]` table, not
  a line inside `[dependencies]`. — Cost if wrong: a confusing manifest error at Task 2.

Ruling: F3 — replaced `default_window_icon().unwrap()` with `.cloned().ok_or(...)?` so a missing
  icon is a setup error rather than a panic on launch. — Cost if wrong: none; `?` is strictly safer
  in a `setup` closure that already returns Result.

Task 1: review verdict Approved with 3 Important findings. Entering fix loop for 1 and 2.
Task 1: PROCESS LESSON — the reviewer noticed task-1-brief.md still contained the INCOMPLETE
  serde attribute (no rename_all_fields). Cause: I extracted the brief BEFORE applying the
  pre-flight fixes to the plan. The implementer only got the correct version because I restated
  it in the dispatch. Fix going forward: always re-extract briefs after editing the plan.
  task-2-brief.md was extracted after the fixes and is current. Re-extract briefs 3-9 fresh.
Task 1: Ruling: ACCEPT findings 1 and 2, fix with two small tests. The wire-format contract
  (Rust -> TypeScript) currently rests on a serde attribute with nothing exercising the
  serialized shape, and the kind/status independence is only half-pinned — the off-diagonal
  combinations (bg+busy, interactive+waiting) are never constructed. Both are exactly the risks
  this review was raised to catch. — Why: the frontend task builds directly on this contract,
  and a break produces no compile error and no failing test in either language. — Cost if
  wrong: two extra tests.
Task 1: Ruling: finding 3 (the report omitted the interruption disclosure it was told to make)
  is NOT sent back for a fix. I caused the interruption and know the provenance first-hand; the
  final state is independently verified. Recording it here instead. — Why: a report rewrite buys
  nothing when the controller already holds the ground truth. — Cost if wrong: none material.
Task 1: minor (deferred): two unrelated `kind` keys at different nesting levels in the JSON
  (session kind vs status tag) — worth a doc comment.
Task 1: minor (deferred): no test for a missing `kind` falling back to "interactive".
Task 1: fix round 1/5 (2 addressed, 0 open; commits 35e033e..360e5e5). The implementer wrote
  all three tests and they passed; a user-requested pause arrived before it committed, so the
  CONTROLLER ran cargo fmt (my snippets were not rustfmt-clean), re-verified 11/11 live tests
  plus fmt and clippy clean, and committed to preserve the work across a machine restart.
  This was a checkpoint, not a controller fix — no logic was written or altered by me.
  STILL OWED: a scoped re-review of 35e033e..360e5e5 before Task 1 can be marked complete.

=== PAUSED at user request (machine restart) ===
RESUME HERE:
  1. Scoped re-review of commits 35e033e..360e5e5 against Task 1 findings 1 and 2.
  2. Then Task 2 (platform traits + three-confirmation liveness). Its brief is current
     (extracted after the plan fixes) at task-2-brief.md.
  3. Re-extract briefs 3-9 fresh before dispatching them — briefs extracted before a plan
     edit go stale silently. Task 1's brief did exactly that.
State: branch feat/tray-and-live-sessions, HEAD 360e5e5, tree clean, 77 tests green
  (66 core + 11 live... actual: 66 lib incl. live, 9 integration, 2 CLI).

=== RESUMED after restart. State verified intact: HEAD 360e5e5, tree clean, 77 tests green. ===
Task 1: re-review verdict — both findings ADDRESSED. Re-reviewer reasoned through the failure
  modes rather than just confirming the tests exist: deleting rename_all_fields makes
  v["sinceMs"] null instead of 5, failing the assertion; and conflating kind with status fails
  on the off-diagonal cases (bg,busy) and (interactive,waiting). Tests-only diff, no production
  code touched.
Task 1: complete (commits c20a27a..360e5e5, review clean after 1 fix round)
Briefs 3-9 re-extracted from the current plan (stale-brief lesson applied).
Task 2: complete (commits 360e5e5..6677262, review clean)
  - Reviewer verified all 5 named risks. Three confirmations enforced as a strict && with each
    independently pinned by its own test; is_alive guards pid <= 0 BEFORE calling kill, so the
    kill(0,..)/kill(-1,..) process-group/broadcast hazard cannot be reached; Command::output
    failure and non-UTF-8 both fail closed; fallback.rs is a total-false stub with no partial
    /proc logic; live.rs gained zero OS knowledge.
  - Reviewer judged Risk 2 (substring match) immaterial: session ids are full UUIDv4 and the
    ps query is scoped to a single pid (`ps -p <pid>`), so it asks "does THIS pid's command
    line contain THIS uuid", not a broad process-table scan.
  - Reviewer ⚠️ (Linux cfg gating verified by reading only — no Linux target installed)
    RESOLVED BY CONTROLLER: installed x86_64-unknown-linux-gnu. Full `cargo check -p perch-core`
    for Linux fails inside libsqlite3-sys (bundled SQLite needs a C cross-compiler) and never
    reaches our code, so I isolated the platform module into a scratch crate with the identical
    mod/pub-use arrangement and checked it on BOTH targets: host OK, Linux OK. The cfg split is
    now verified mechanically, not by inspection.
Task 2: minor (deferred): macos.rs's real fail-closed paths (ps missing, non-UTF-8, kill error)
  are correct by inspection but pinned by no test — the FakeProbe tests exercise the AND logic
  only. A platform-specific integration test would close this.
Task 2: minor (deferred): the sort test's discriminating power relies on write_record hardcoding
  name:"n" for every fixture. If that helper is ever parameterized, the test could stop
  discriminating rank-first from name-only while still passing.
Task 2: FORWARD CONCERN for Tasks 4/9: CI's ubuntu-latest job runs `cargo test --workspace`,
  which today genuinely compiles perch-core with the Linux fallback — good. But Task 4 adds
  src-tauri to the workspace, and Tauri needs system libs (webkit2gtk et al) on Linux. The
  Linux job will break unless src-tauri is excluded from it or those deps are installed.
  Task 9 must handle this deliberately rather than discovering it in CI.

Task 3: gate FAILED — and it found a real SPEC defect, not a code defect. Excellent catch by
  the implementer, which it correctly escalated rather than patching Task 2's design itself.
CONTROLLER VERIFICATION of the finding: both live records (pids 3375, 3763, both
  kind=interactive) have command lines of exactly `claude --dangerously-skip-permissions` —
  no session id on argv. Earlier in this project I saw `--session-id` on pid 30778, but that
  was a `kind: bg` session spawned by the daemon. So spec §7's confirmation 3 held only for
  bg sessions and filtered out every interactive one — i.e. almost everything the user cares
  about. The CLI printed 0 sessions while 2 were genuinely running.
Ruling: REDEFINE confirmation 3 as "the process's executable name is `claude`", verified via
  `ps -o comm=` (executable only, no args) rather than `ps -o command=`. Reasoning: (a) records
  are keyed by pid — `<pid>.json` — so a new session on a recycled pid OVERWRITES the stale
  record and two records can never claim one pid; the only residual hazard is a NON-Claude
  process inheriting the pid, which an executable-name check fully excludes. (b) matching on
  `command=` for the substring "claude" would false-positive on any unrelated process with a
  path like `claude-dashboard` in its arguments (e.g. `vim ~/projects/claude-dashboard/x.rs`);
  `comm=` returns just `claude` and is immune. Verified on the real machine: comm= is exactly
  "claude" for both live sessions. — Cost if wrong: a non-Claude process named `claude`
  inheriting a recycled pid, which is vanishingly unlikely and self-corrects on next scan.
Ruling: the trait method becomes `process_name(&self, pid: i32) -> Option<String>` rather than
  `cmdline_contains(pid, needle)`. — Why: returning the name makes the probe a plain fact
  reporter and moves the policy ("is it claude?") into live_sessions where it is testable and
  visible, instead of hiding a substring policy inside the OS layer.
Ruling: ADD `SessionStatus::Idle`. Real records carry `status: "idle"`, which the spec never
  listed; Task 1 maps unknown statuses to Working, so an idle session would render green as
  though mid-task. Any FURTHER unknown status still falls back to Working. — Cost if wrong:
  none; it is strictly more faithful to the data.
Spec §7 corrected and committed (f2c5240) with both changes and the reasoning. Plan updated.
Task 2: fix complete (1189916). Controller re-ran Task 3's gate: 3 of 3 records now listed
  (was 0 of 2). The corrected liveness rule works on real data.
Task 3: gate re-run surfaced TWO more real-data defects, both in the CLI display layer:
  1. A brand-new session (pid 8138) carries NO `status` and NO `statusUpdatedAt` at all —
     both absent, not merely unrecognised. i64_at returns 0, so `now - 0` renders as
     "496716h" (~56 years, i.e. the epoch). A missing timestamp must not be formatted as a
     duration. This is a display bug, not a parse bug: SessionMeta correctly reports 0.
  2. personal-dashboard-73 has `status: "idle"` and the CLI prints "working" — the Sessions
     arm's `_ => "working"` fallback predates SessionStatus::Idle. The parser is right; the
     renderer was never updated.
Ruling: fix both in perch-cli (Task 3's code), not in perch-core. `human_elapsed` stays a pure
  duration formatter; the caller decides that an absent timestamp has no duration to show and
  renders an em dash. — Why: 0 is a legitimate duration; only the CALLER knows the timestamp
  was missing rather than genuinely zero. Pushing the guard into the formatter would conflate
  "no data" with "zero elapsed". — Cost if wrong: an em dash where a real duration was wanted.
Ruling: Task 8's popover must carry the same two guards. Noted in the plan so the UI does not
  independently rediscover a 56-year timer.
Task 3: fix complete (9c529c6). Implementer honestly reported gate checks 2 and 3 as N/A —
  no waiting session existed on the machine during either run.
  CONTROLLER RESOLVED both: built a synthetic sessions dir under the scratchpad and drove the
  CLI with --config-dir, using the REAL live claude pids so all three liveness confirmations
  genuinely pass and only the STATUS field is synthetic. ~/.claude never touched (verified).
    check 2 PASS — "waiting · dialog open" with elapsed "32h" from statusUpdatedAt.
    check 3 PASS — and more strongly than the unit test: the fixtures are alphabetically
      INVERTED (zzz-blocked-session vs aaa-busy-session), so the blocked session sorting first
      proves rank beats name rather than coinciding with it. This is exactly the hardening the
      Task 2 reviewer suggested for the unit test, now demonstrated end-to-end on real pids.
  Incidentally reproduces the 32h-blocked-session scenario that motivated the product.
Task 3 + Task 2 fix: review Approved, with one Important documentation finding against MY spec.
Ruling: ACCEPT and fix immediately (2df6c66). Spec §7 bullet 3 still read "The process command
  line identifies it as a Claude Code process" — but the entire point of the correction was to
  stop matching on the command line. A future reader implementing from that bullet would have
  reintroduced the exact argv substring bug. Now reads "The process's executable name — not its
  arguments — is `claude`", matching the plan and the code. — Why: a spec that misdescribes its
  own implementation is worse than one that is silent; this is the second time this bullet has
  been wrong. — Cost if wrong: none, it now matches the shipped code verbatim.
Ruling: ACCEPT the reviewer's Minor #1 as a correction to my reasoning, and soften the spec. My
  claim that the executable-name check "fully excludes" the residual hazard was OVERSTATED. The
  reviewer identified a real window: session A crashes leaving a stale record; the OS reuses its
  pid for a genuinely new `claude` process B; B has not yet written its own record. Every check
  passes — alive, and really is a Claude process — so A's stale name/cwd is briefly shown
  against B's pid. It self-corrects when B writes its record (overwriting A's). Spec now states
  this plainly and names `procStart` as the fix if it ever proves observable. — Why: I was
  wrong, and an overstated safety claim in a spec is how the next person stops looking.
  — Cost if wrong: none; this is strictly more honest.
Task 3: minor (deferred): the CLI's status-label match (Idle vs Working) has no unit test — the
  fix rests on the gate transcript. Extracting a testable label fn (mirroring elapsed_or_dash)
  would close it cheaply. Flag to the whole-branch review.
Task 3: minor (deferred): the table header prints even when there are no live sessions.
Task 2: complete (commits 6677262..1189916, review clean)
Task 3: complete (commits 829f886..9c529c6, review clean after 1 fix round)

Task 4: complete pending review (commit b94b603).
  PROCESS ADAPTATION: two agents were killed by the stall watchdog on this task, both during
  multi-minute compiles (cargo install tauri-cli, cargo tauri icon). I absorbed the long
  operations myself — installed tauri-cli 2.11.4, generated the placeholder icons — then
  dispatched a FRESH agent for the three remaining edits with an explicit no-builds rule, and
  ran every build and gate as controller. Verification was already the controller's job, so
  this removed the failure mode without writing implementation code or skipping review.
  Controller verification at b94b603:
    cargo build --workspace ....... OK (tauri 2.11.5 + perch-app compile)
    cargo test --workspace ........ 87 pass (5 cli + 73 core + 9 integration)
    cargo fmt --all --check ....... clean
    cargo clippy --workspace ...... 0 warnings
    pnpm build .................... OK (190.91 kB js, 60.20 kB gzip)
    cargo test -p perch-core -p perch-cli (simulating CI's new Linux scope) .... 87 pass
  git hygiene verified: node_modules/ and dist/ ignored, pnpm-lock.yaml committed.
  Icons: placeholder, generated by cargo tauri icon from a programmatically drawn 1024x1024
  source (/tmp/perch-icon-source.png) — dark rounded ground, amber bar, green dot. Replaceable.
  CI now scopes by platform via matrix.cargo-scope: macOS --workspace, Linux
  "-p perch-core -p perch-cli", with the webkit2gtk reasoning in a YAML comment. The forward
  concern logged at Task 2 is therefore closed HERE rather than deferred to Task 9.
Task 4: complete (commits 2df6c66..faa4f82, review clean). No scope creep — reviewer grepped
  and confirmed zero tray/invoke_handler/#[tauri::command]/watcher code. CI matrix split judged
  correct and non-weakening.
Task 4: reviewer SHARPENED my no-network concern: the Cargo.lock scan partially backstops NAMED
  crates in src-tauri, but only while the committed lockfile is in sync (the job never builds).
  A hand-rolled socket in src-tauri/src is covered by NOTHING. Deferring to Task 9 accepted,
  but the gap is now written into the plan's Task 9 text verbatim so it cannot be missed, and
  Tasks 5-7 dispatches will carry an explicit "no network code in src-tauri/src" constraint to
  cover the four-task window.
Task 4: minor -> ACTED ON: reviewer flagged `"csp": null` as fine for an empty window but wrong
  once real data renders. Added an explicit instruction to Task 8 (where session names, cwds and
  statuses first reach the DOM) to set `default-src 'self'` and confirm the popover still loads.
Task 4: minor (deferred): CI never type-checks the frontend — `vite build` does not run
  `tsc --noEmit`, so tsconfig's strict/noUnusedLocals are declared but unenforced. Reviewer
  hand-checked the three current src/ files as clean. Flag to the whole-branch review.
Task 4: note: notify/chrono/serde/serde_json/anyhow are declared in src-tauri/Cargo.toml but
  unused until Tasks 5-8. Not scope creep — manifest declaration only, confirmed by grep.
Task 5: implemented and committed (b370d22). set_activation_policy resolved cleanly under
  Tauri 2.11.5 — the one API the dispatch flagged as most likely to have moved.
  Controller gates: fmt clean, 0 clippy warnings, 0 network-code hits in src-tauri/src.
  Verified markers: tray id "perch-tray" present (Task 9 depends on it), window label
  "popover" matches tauri.conf.json:15 (a mismatch would make the tray click silently no-op).
  CONTROLLER INTERACTIVE VERIFICATION (ran ./target/debug/perch-app):
    launches and stays alive ......................... PASS
    0 windows visible at startup ..................... PASS (visible:false honoured)
    no Dock tile / not in the app switcher ........... PASS — perch-app is in the process
      list but absent from `every process whose visible is true`, i.e. Accessory policy
      is genuinely in effect, not merely requested in code.
    tray item appears ................................ NEEDS HUMAN (counting menu bar extras
      requires granting osascript assistive access; declined to alter the user's machine)
    left-click toggles popover ....................... NEEDS HUMAN
    right-click menu Quit exits ...................... NEEDS HUMAN
    Cmd-W hides rather than quits .................... NEEDS HUMAN
  Left the app running for the user to eyeball; quitting it via the tray menu doubles as the
  check for the tray item existing and Quit working.
Task 5: HUMAN VERIFICATION by the user. Results:
  (a) tray item appears ......................... PASS
  (b) no Dock icon / not in app switcher ........ PASS (controller-verified)
  (c) left click opens ... "a white rectangle in the middle of the screen" .... FAIL
  (d) right-click menu shows Quit ............... PASS
  (e) Cmd-W ..................................... "does nothing"
Ruling: (c) is a genuine product defect and MY plan's gap. The spec calls this a popover
  (§9.1) and a popover must anchor under its tray icon; Task 5 never specified positioning, so
  the window defaulted to centre-screen. Added Step 1b: tauri-plugin-positioner with the
  tray-icon feature, on_tray_event forwarding so the plugin knows where the icon is, and
  move_window(Position::TrayBottomCenter) BEFORE show() so it does not paint centre-screen and
  jump. — Cost if wrong: a plugin dependency; it is first-party Tauri and carries no network.
Ruling: also add hide-on-focus-loss in the same step. The user did not report this, but a
  popover that survives clicking away is just a window — and once it is anchored to the tray,
  staying open while you work elsewhere would be worse, not better. — Cost if wrong: a user
  who wants it to persist would find it dismissive; reversible in one line.
Ruling: (e) Cmd-W doing nothing is CORRECT, not a defect, and my plan's checklist was wrong to
  expect otherwise. The window has decorations:false and no menu, so Cmd-W is never bound —
  there is no close to intercept. Dismissal is: click the tray again, or click away once
  Step 1b lands. prevent_close still guards a programmatic close. Checklist corrected. — Cost
  if wrong: none; this documents observed behaviour rather than asserting intended behaviour.
Note: the white rectangle itself is expected — App.tsx is still the Task 4 placeholder. Task 8
  brings the dark dense popover. Not a defect.
Task 8 note: add Escape-to-dismiss in the frontend, now that Cmd-W is confirmed unavailable.
Task 5: user re-test after 83fe4d6 — popover anchors under the tray icon PASS, dismisses on
  click-away PASS. But: "only in desktop with no maximized application" — it does NOT appear
  when a fullscreen app is frontmost.
Task 5: 814a0c8 tried set_visible_on_all_workspaces(true). User re-tested: "no still the same".
  The implementer had already traced this from source rather than guessing: tao 0.35.3's
  set_visible_on_all_workspaces (platform_impl/macos/window.rs:1539) sets ONLY
  NSWindowCollectionBehaviorCanJoinAllSpaces; FullScreenAuxiliary appears nowhere in tao. The
  user's test confirms CanJoinAllSpaces alone is insufficient for a fullscreen Space.
Ruling: ADD the objc2 collection-behaviour fix. Normally a new dependency on a public repo is
  my call to weigh carefully — but `objc2-app-kit` 0.3.2 is ALREADY in the dependency graph
  (Tauri pulls it transitively, verified in Cargo.lock alongside objc2 0.6.4 and 11 sibling
  crates). Declaring it directly adds zero new crates to the build and no new supply-chain
  surface; it is already compiled. — Why: a menu-bar popover that silently fails to appear
  whenever a fullscreen app is frontmost is a serious product defect, not a nicety; many people
  work fullscreen all day and it would read as "the app is broken". — Cost if wrong: an unsafe
  pointer cast to NSWindow, macOS-gated, ~8 lines, reversible; and if the cast were wrong the
  failure is loud (crash on launch), not silent.

Task 5: SYSTEMATIC DEBUGGING of "popover does not appear over fullscreen apps". Two fixes had
  failed (set_visible_on_all_workspaces; then CanJoinAllSpaces|FullScreenAuxiliary via objc2),
  so per process: no third guess, instrument every boundary instead. Throwaway eprintln probes
  in lib.rs, never committed, reverted after.
  EVIDENCE (probe builds 2-4, user-driven, desktop vs fullscreen):
   - Tray click ARRIVES in fullscreen. move_window Ok. show Ok, is_visible=true. set_focus Ok.
     Focused(true) fires and HOLDS ~2s (so hide-on-blur is NOT the cause — hypothesis refuted).
   - Read-back at click time: collectionBehavior=257 (both flags present and surviving),
     level=5, occlusion=Visible, isVisible=true — but isOnActiveSpace=FALSE.
   - Desktop baseline: isOnActiveSpace=TRUE immediately and at +500ms.
     Fullscreen: FALSE immediately and STILL FALSE at +500ms — persistent, not a stale read.
   - Single display (nscreens=1, 2560x1440), so no per-display-Spaces confusion.
   - Probe 4: show via orderFrontRegardless with NO activation (appActive=false, isKey=false
     confirmed) — isOnActiveSpace STILL false. Activation hypothesis refuted.
  ROOT CAUSE: the window server refuses to place this plain NSWindow on a fullscreen Space
  regardless of collection-behaviour flags or activation. Working-example comparison
  (tauri-nspanel's dedicated fullscreen example: "must be non-activating" + full_screen_auxiliary
  + can_join_all_spaces) shows the remaining difference is the window CLASS — macOS honours
  these behaviours on NSPanel; the plugin subclasses the window into one at runtime.
Ruling: adopt tauri-nspanel (git dep, branch v2.1 — not on crates.io per its docs). Trade-off
  recorded honestly: a git dependency in a public repo is less tidy than a crates.io one, but
  Cargo.lock pins the exact commit so builds stay reproducible; it is the widely-used Tauri
  community plugin for precisely this problem by a known maintainer; and the alternative is
  hand-rolling the same object_setClass swizzle plus key-window overrides in unsafe code, which
  is strictly riskier. — Cost if wrong: an upstream branch move breaks a fresh clone until the
  rev is pinned explicitly; mitigate by pinning `rev` rather than `branch` once it works.
Task 5: USER CONFIRMED after 1d80103 (NSPanel subclass): "now it's ok" — popover appears over a
  fullscreen app. Root cause proven by fix: the window CLASS. "empty white" = expected placeholder
  until Task 8.
  Fix attempts: 1) set_visible_on_all_workspaces — failed. 2) objc2 CanJoinAllSpaces|
  FullScreenAuxiliary on NSWindow — failed. [systematic debugging: probes, 2 hypotheses refuted]
  3) NSPanel via tauri-nspanel, non-activating + same behaviours — PASS.
  Implementer read the plugin's v2.1 source before coding; chose show_and_make_key over show()
  after confirming show() never calls makeKeyWindow (Task 8's Escape needs key). macOS-gated the
  dep (crate cannot compile elsewhere). Lockfile: one new crate (pastey 0.2.3), no churn.
Security review flagged the git dependency (supply chain). Controller pinned it by rev
  c9ec2130422200f0863b23dfdad02b133a529b07 instead of branch v2.1 (commit above), as the ledger
  ruling had pre-planned. Build verified after the pin.
Task 5: review Approved (7 commits, e5e1e27..e65b8a5). Reviewer verified every API claim against
  the vendored plugin checkout at the pinned rev (HEAD == c9ec213); confirmed show() does only
  orderFrontRegardless while show_and_make_key adds makeKeyWindow (panel.rs:242 vs :409), so the
  implementer's deviation was source-justified; confirmed maximize()/fullscreen() never called;
  confirmed dead code from attempts 1-2 fully removed (no objc2-app-kit dep, no stray unsafe);
  audited pastey 0.2.3 (proc-macro only, zero deps, no I/O); confirmed macOS gating is
  NECESSARY (the plugin `pub use objc2_app_kit` unconditionally and cannot compile elsewhere).
Task 5: Important (open, pending human) — dismiss-on-blur and anchoring were human-verified only
  BEFORE the NSPanel conversion. Reviewer traced tao's window_delegate.rs + the plugin's
  from_window (object_setClass in place, delegate untouched, resignKeyWindow not overridden) and
  judges both survive — strong inference, not proof. User re-test requested (app relaunched).
Task 5: minor -> ACTED ON: plugin docs warn a drag region + double-click-maximize crashes a
  non-activating fullscreen-auxiliary panel unless core:window:deny-internal-toggle-maximize is
  set. Unreachable today (no drag region, resizable:false). Added a guard note to Task 8.
Task 5: minor (recorded): tauri-nspanel depends on tauri with the `macos-private-api` feature,
  which Cargo feature-unification enables workspace-wide. Inherent to the plugin; note for any
  future private-API audit.
Task 5: Ruling: PARK the open Important (post-NSPanel dismiss-on-blur not human-re-tested). User
  said "go ahead" without answering. Reviewer's source trace (delegate untouched by
  object_setClass; resignKeyWindow not overridden; move_window goes through set_position) is
  strong evidence it works. It WILL be exercised by Task 8's mandatory hand-verification, which
  opens the real popover and dismisses it — so this is deferred to a checkpoint 3 tasks away, not
  dropped. — Cost if wrong: a one-line change in on_window_event, found at Task 8.
Task 5: complete (commits e5e1e27..e65b8a5, review clean, 1 parked)

Task 6: complete (commits b55fe94..36fde14, review clean)
  - Reviewer traced read-only end to end: db_path derives from $HOME/Library/Application Support
    only, never from config_dir/CLAUDE_CONFIG_DIR; all perch-core write paths reachable from
    reindex go through Db; fs writes in discovery.rs/scan.rs exist only under #[cfg(test)].
  - No message text can cross the boundary; no unwrap/expect in reachable command paths.
  - IndexStats edit is exactly one derive; Default/PartialEq/Eq untouched.
Task 6: minor -> ACTED ON: `today` is a trailing-24h window, not a calendar day. Ruling: keep
  the API field name for now; Task 8 must label the tile "24h" rather than the mockup's "Today"
  — the honest name for what the number is. Added to the Task 8 brief.
Task 6: minor (deferred): HOME="" (empty, not unset) would make db_path relative to cwd. Extreme
  edge; the realistic failure (unset) is handled. Hardening candidate for the platform-paths
  milestone.

Task 7: review Approved, 2 Important findings, both against code MY brief mandated verbatim.
Ruling: ACCEPT #1 (debounce dead-window) and fix. The reviewer corrected the implementer's
  "rare" framing with arithmetic I agree with: `last` resets after EVERY emit including each 5s
  poll emit, so a ~250ms dead window recurs at the start of every cycle — roughly a 1-in-20
  chance per status transition of a full ~5s delay. For the product's headline signal ("this
  session is waiting on you") that is a rough edge worth removing when the fix is a few lines:
  trailing-edge re-emit — after a dropped event, wait only until the debounce boundary, not the
  poll boundary. — Cost if wrong: one extra emit per burst; harmless.
Ruling: ACCEPT #2 (silent exits) and fix. lib.rs already logs non-fatal setup failures via
  eprintln!; the watcher's three silent returns (no config dir, watcher build failed, watch
  failed) leave a misconfigured user with an app that "just never updates". Log each. — Cost if
  wrong: three stderr lines nobody reads.
Task 7: minor (recorded): reviewer corrected MY risk framing — is_alive uses libc::kill (a
  syscall), so the poll spawns ONE /bin/ps per live session per tick, not two. Cheaper than I
  claimed.
Task 7: fix round 1/5 (2 addressed, 0 open; commits 17a2022..bc0ae10). Re-reviewer hand-traced
  four scenarios: trailing-edge emit lands ~250ms after the first event; zero-wait edge cannot
  spin (every exit path clears pending and resets last); 50ms event storm coalesces to one emit
  per 250ms without starvation; 5s poll backstop unchanged. Constants, initial emit, and the
  Disconnected exit all verified untouched.
Task 7: complete (commits c6821f5..bc0ae10, review clean after 1 fix round)

Task 8: review Approved (3dbb902); one ⚠️ became a real defect on inspection: NO capabilities
  file existed, generated ACL was `{}`. In Tauri 2 app commands work ungated but core APIs do
  not — listen('sessions-changed') and window.hide() would silently no-op, breaking live
  updates and Escape. Root cause: MY Task 4 scaffold brief never created capabilities/.
Task 8: fix round 1/5 (1 addressed, 0 open; commits 3dbb902..65ec50c). capabilities/default.json
  scoped to windows:["popover"], permissions: core:event:default, core:window:allow-hide,
  core:window:deny-internal-toggle-maximize. NOT core:default wholesale. Re-reviewer audited
  every frontend call → exactly one grant each; verified allow-hide is absent from
  core:window:default (30 perms, all reads). Generated ACL now ['popover-default'].
Task 8: minor (recorded): StatusDot checks kind=='bg' before status=='idle', so a bg+idle
  session shows the grey dot — verbatim from my brief; Background/Ended are never produced by
  the parser today. Harmless; note for the whole-branch review.
Task 8: minor (deferred): pre-existing tsc --noEmit failure on main.tsx (missing vite/client
  types) — inert against `pnpm build`; a one-line vite-env.d.ts closes it. Pairs with the Task 4
  deferred "CI never type-checks the frontend".
Task 8: complete pending the user's visual test of 65ec50c (relaunched as pid 16309). If that
  test fails, it reopens as a fix round; Task 9 does not touch App.tsx or capabilities.

Task 9: implemented (6f1a2fc). Tray title via tray_by_id/set_title — implementer verified in
  tauri 2.11.5 source that set_title dispatches to the main thread internally, so calling it
  from the watcher thread is safe. All src-tauri guards proven to fire with self-tests.
Task 9: BLOCKING FINDING by the implementer, found by RUNNING the no-network script instead of
  merely editing it: the Cargo.lock grep has been failing since Task 4 (b94b603). Controller
  verified root cause: tauri 2.11.5 declares reqwest/hyper for its Android/iOS targets; a
  lockfile lists every platform's deps; `cargo tree -i reqwest -p perch-app` on the host prints
  NOTHING, `--target all` shows them under tauri. False positive, not a real dependency.
  MY ERROR, two layers: (1) the lockfile grep was designed when the workspace was perch-core
  only and is too coarse once Tauri enters; (2) in the Task 4 dispatch I told the reviewer "I
  have already confirmed no HTTP or telemetry crate resolves into the graph" — that claim was
  from the Task 1-era lockfile and was STALE by the commit it was attached to. CI has been red
  on the no-network job for five tasks and nobody noticed because there is no remote yet.
Ruling: REPLACE the lockfile grep with a `cargo tree` check on the real host graph:
  `cargo tree -i <crate> -p perch-core -p perch-cli -p perch-app` for each forbidden crate,
  failing if any is reachable. This keeps the transitive-catch property (cargo tree resolves the
  actual graph, not the manifests) while excluding platform-gated deps we never build. It must
  run on macOS (where perch-app resolves); on Linux, scope to perch-core + perch-cli. Keep a
  positive-control self-test: `cargo tree -i serde -p perch-core` must succeed, proving the
  probe finds crates that ARE present. — Why: a guard that is red on a clean tree trains
  everyone to ignore it, which is worse than no guard. — Cost if wrong: cargo tree is slower
  than grep (seconds, not ms) and needs a resolved workspace; acceptable in CI.
Task 9: fix round 1/5 (lockfile grep -> cargo tree resolved-graph check; commit eeb46f0).
Task 9: review Approved. Reviewer independently reproduced the Tauri mobile-target false
  positive; verified tray_by_id is inherent (app.rs:803) and set_title dispatches to the main
  thread via run_item_main_thread! (tray/mod.rs:538); confirmed cargo tree exits 0 whether a
  crate is present or absent, so the script correctly greps stdout; confirmed `^hyper ` does
  not match hyper-util; hand-ran every guard and every self-test. One Important:
Task 9: Ruling: ACCEPT the Important and fix (round 2). The resolved-graph self-test hardcoded
  `-p perch-core` while the real loop uses ${{ matrix.cargo-scope }} — a malformed scope would
  empty every cargo tree stdout and read as "clean". Every other self-test in the diff reuses
  the real check's variable; this one must too. — Cost if wrong: none; one token.
Task 9: minor (recorded): commands.rs write-guard exclusion is precautionary — that file has
  no fs calls today (db::open does them, already excluded). Correct, just not load-bearing yet.
Task 9: minor (deferred): the OS/cargo-scope matrix `include:` block is duplicated verbatim
  between the test and no-network jobs. DRY nit; a YAML anchor would fix it.
Task 9: fix round 2/5 (1 addressed, 0 open; commits eeb46f0..3d32043). Self-test now uses the
  identical ${{ matrix.cargo-scope }} as the production loop; mangled scope exits 1. Re-reviewer
  clarified the zsh/bash note: GH Actions substitutes ${{ }} textually before the shell runs,
  so CI never depends on word-splitting — the concern was valid for local reproduction only.
Task 9: complete (commits 65ec50c..3d32043, review clean after 2 fix rounds)

ALL 9 TASKS COMPLETE. Task 8's user visual test of 65ec50c still outstanding (app running as
pid 16309). Proceeding to whole-branch review; the visual result will be surfaced at finish.

TASK 8 VISUAL TEST: user reports "still a white rectangle without anything". ROOT CAUSE FOUND
  and it is the CONTROLLER'S testing method, not the UI: a DEBUG Tauri build embeds devUrl
  (http://localhost:1420) and expects `cargo tauri dev` to have started Vite. Verified: the
  debug binary contains the string localhost:1420 and nothing listens on that port → the webview
  gets connection refused → blank white. EVERY white rectangle since Task 5 was this. The real
  UI has never been rendered. A RELEASE build embeds frontendDist (dist/) — that is what ships
  and what should have been launched. Rebuilding release now.
  Consequence for the record: Task 5's human checks (tray, anchoring, fullscreen, dismiss) were
  all genuinely about the window and remain valid; only "what is inside the window" was never
  actually tested. Task 8's visual verification restarts from zero on the release binary.
ROOT CAUSE, precisely (tauri 2.11.5 build.rs:256-259 + tauri-codegen context.rs:178): the
  `dev` cfg is `!custom_protocol`; codegen uses devUrl when dev. `cargo tauri build` enables the
  `custom-protocol` feature; plain `cargo build` (debug OR release) does not. The standard Tauri
  scaffold declares `[features] custom-protocol = ["tauri/custom-protocol"]` in
  src-tauri/Cargo.toml; MY Task 4 brief omitted it. So no binary launched in this milestone has
  ever embedded dist/. Verified: even `cargo build --release` still carries localhost:1420.
Ruling: (a) for testing NOW, build with `--features tauri/custom-protocol` — no repo change;
  (b) the repo fix — add the [features] section, and make CI's `app` job build WITH the feature
  so it proves the SHIPPABLE artifact compiles, not the dev-only variant — goes into the final
  fix wave after the whole-branch review returns, alongside whatever it finds.

WHOLE-BRANCH REVIEW: verdict "Fix before merge". 0 Critical, 4 Important, plus fix-before-merge
  promotions among the deferred items. Cross-cutting findings no per-task review could see:
  #1 watcher.rs:33-36 — if <config>/sessions is absent at launch, notify returns
     path_not_found and the thread RETURNS, taking the 5s poll backstop with it. A fresh user
     gets a tray that never updates until restart. Ruling: ACCEPT — fall back to poll-only on
     watch() failure and retry watch() on each tick.
  #2 commands.rs + App.tsx — usage_summary reads ~/Library/Application Support/Perch/index.db,
     which NOTHING populates: the app never invokes `reindex`, and perch-cli writes to a
     different file (./perch-index.db). The popover shows `0 / $0.00 est` — which reads as
     "you spent nothing", a fabricated-looking number. And getUsageSummary runs once on mount,
     never refreshed. Ruling: ACCEPT — call reindex on mount then fetch usage; re-fetch usage
     on every sessions-changed; add `hasData` (turn_count > 0) to UsageSummary and render `—`
     when false. Real numbers, honestly refreshed, honest empty state.
  #3 ci.yml — the frontend is an outbound-capable surface enforced only by the CSP string; CI
     greps Rust only. A `connect-src https://…` edit would pass. Ruling: ACCEPT — assert the CSP
     value in tauri.conf.json and grep src/ + index.html for fetch(/WebSocket/XMLHttpRequest/
     https?://, with self-tests.
  #4 README — no mention of the app, pnpm/Tauri prerequisites, the rev-pinned git dependency
     (tauri-nspanel), or that a Finder-launched .app does not inherit CLAUDE_CONFIG_DIR from a
     shell rc. Ruling: ACCEPT — for a privacy-led public repo the one non-crates.io dependency
     deserves a sentence.
  Promoted from deferred → fix now: vite-env.d.ts + a `tsc --noEmit` CI step (one error today);
  remove the commands.rs write-guard exclusion (no writes there; it un-audits the likeliest
  file to gain one).
  ADDED BY CONTROLLER to the same wave: `[features] custom-protocol = ["tauri/custom-protocol"]`
  in src-tauri/Cargo.toml (my Task 4 scaffold omission — without it no build embeds the
  frontend), and CI's `app` job must build WITH that feature so it proves the shippable artifact.
  Cheap minors folded in: pid tiebreaker in the live sort; UI labels idle as "idle"; delete the
  stale duplicate paragraph in spec §7; fix plan line 22's superseded hazard wording; one-line
  §9.1 note recording the count-vs-percentage deviation.
  Deferred (acceptable): all other minors per the reviewer's triage.
  NEW human check from the reviewer: click the tray icon WHILE the popover is open — if the
  panel resigns key on the status-item click, Focused(false) hides it before toggle runs, which
  re-shows it (flicker; cannot close via tray). Added to the user's test list.
Ruling: ONE fix wave covering all of the above, then ONE scoped re-review, per process.
FINAL FIX WAVE re-review: ALL 9 items ADDRESSED (commits 3d32043..e2eff2e). Re-reviewer
  hand-traced the poll-only watcher loop (no busy loop, retry once per tick, never creates the
  dir); confirmed reindex-failure still reaches getUsageSummary; hand-verified the CSP assertion
  rejects a widened policy; verified the extended sort test discriminates rank, name AND pid.
  87 tests reproduced clean.
Out-of-scope (recorded, deferred): README's "one outbound host: anthropic.com" is aspirational
  today (the app makes no requests) — add "in a future release" wording later. reindex runs
  once per popover-process lifetime; a long-running Perch shows stale totals until restart —
  follow-on: reindex on each show, or on a timer.
BRANCH IS REVIEW-CLEAN. Outstanding before finish: user's visual checks on pid 46578, and the
  user's new requirement "menu bar must stay visible while the popover is open (like
  CodexBar)". Root cause: a detached NSPanel does not hold the fullscreen menu bar; only an
  ACTIVE status item does. tray-icon exposes ns_status_item() but Tauri's TrayIcon keeps it in
  a private field with no accessor. Spike: find our own NSStatusBarWindow via NSApp.windows and
  highlight its button while the popover is open. Throwaway probe, not committed.
USER FEATURE REQUEST (pending yes): richer popover — project/kind/version per row, per-session
  tokens+cost from the index, a Recent section (last 3 ended sessions), taller window.
MENU-BAR SPIKE RESULT: refuted. The probe found and highlighted the real NSStatusBarButton
  (log: "NSStatusBarWindow: button NSStatusBarButton -> highlight(true)") and the fullscreen
  menu bar STILL retracted when the mouse left it; the popover correctly stayed open.
  Conclusion: highlight state does not hold the bar; only a status-item-owned NSPopover (or a
  tracked NSMenu) does. That is a proper task (host the webview in an NSPopover anchored to the
  button found via NSStatusBarWindow), not a pre-merge fix. Ruling: merge the milestone first;
  queue "menu bar stays visible while open" as the next task alongside the richer popover.
  Probe reverted; nothing committed.
TASK 8 VISUAL SIGN-OFF by the user on e2eff2e (release, custom-protocol): dark popover renders;
  Escape closes it: YES; live update on session start/quit: YES; tray click while open: OK;
  stats show numbers (index populated on mount): YES. Parked Task 5 dismiss check: covered.
Task 8: complete. MILESTONE 2 COMPLETE, review-clean, human-verified.
Queued after merge: (1) menu bar must stay visible while the popover is open (status-item-owned
  NSPopover); (2) richer popover (per-row project/kind/version, per-session tokens+cost,
  Recent section, taller window) — pending the user's yes.
