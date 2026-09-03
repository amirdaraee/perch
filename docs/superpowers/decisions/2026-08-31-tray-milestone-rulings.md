# Rulings — tray & live sessions milestone (2026-08-31 → 09-03)

Every decision the controller took on the user's behalf during this milestone, in order,
extracted from the execution ledger. Each states what was decided, why, and the cost if wrong.

- Ruling: F1 — `#[serde(tag = "kind", rename_all = "camelCase")]` renames enum VARIANTS only, not the
- Ruling: F2 — clarified that the libc dependency is a `[target.'cfg(unix)'.dependencies]` table, not
- Ruling: F3 — replaced `default_window_icon().unwrap()` with `.cloned().ok_or(...)?` so a missing
- Task 1: Ruling: ACCEPT findings 1 and 2, fix with two small tests. The wire-format contract
- Task 1: Ruling: finding 3 (the report omitted the interruption disclosure it was told to make)
- Ruling: REDEFINE confirmation 3 as "the process's executable name is `claude`", verified via
- Ruling: the trait method becomes `process_name(&self, pid: i32) -> Option<String>` rather than
- Ruling: ADD `SessionStatus::Idle`. Real records carry `status: "idle"`, which the spec never
- Ruling: fix both in perch-cli (Task 3's code), not in perch-core. `human_elapsed` stays a pure
- Ruling: Task 8's popover must carry the same two guards. Noted in the plan so the UI does not
- Ruling: ACCEPT and fix immediately (2df6c66). Spec §7 bullet 3 still read "The process command
- Ruling: ACCEPT the reviewer's Minor #1 as a correction to my reasoning, and soften the spec. My
- Ruling: (c) is a genuine product defect and MY plan's gap. The spec calls this a popover
- Ruling: also add hide-on-focus-loss in the same step. The user did not report this, but a
- Ruling: (e) Cmd-W doing nothing is CORRECT, not a defect, and my plan's checklist was wrong to
- Ruling: ADD the objc2 collection-behaviour fix. Normally a new dependency on a public repo is
- Ruling: adopt tauri-nspanel (git dep, branch v2.1 — not on crates.io per its docs). Trade-off
- Task 5: Ruling: PARK the open Important (post-NSPanel dismiss-on-blur not human-re-tested). User
- Task 6: minor -> ACTED ON: `today` is a trailing-24h window, not a calendar day. Ruling: keep
- Ruling: ACCEPT #1 (debounce dead-window) and fix. The reviewer corrected the implementer's
- Ruling: ACCEPT #2 (silent exits) and fix. lib.rs already logs non-fatal setup failures via
- Ruling: REPLACE the lockfile grep with a `cargo tree` check on the real host graph:
- Task 9: Ruling: ACCEPT the Important and fix (round 2). The resolved-graph self-test hardcoded
- Ruling: (a) for testing NOW, build with `--features tauri/custom-protocol` — no repo change;
- gets a tray that never updates until restart. Ruling: ACCEPT — fall back to poll-only on
- never refreshed. Ruling: ACCEPT — call reindex on mount then fetch usage; re-fetch usage
- greps Rust only. A `connect-src https://…` edit would pass. Ruling: ACCEPT — assert the CSP
- shell rc. Ruling: ACCEPT — for a privacy-led public repo the one non-crates.io dependency
- Ruling: ONE fix wave covering all of the above, then ONE scoped re-review, per process.
- button found via NSStatusBarWindow), not a pre-merge fix. Ruling: merge the milestone first;
