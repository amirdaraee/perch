# Contributing to Perch

Thanks for looking. Before you write any code, please read the **Invariants**
below. They are the unusual part of this project, they are enforced by CI, and
they are not guessable from reading the source.

## Invariants

These are not style preferences. A pull request that breaks one of them will be
sent back even if it is otherwise excellent, because each of them is a promise
Perch makes to the people who install it.

### 1. Perch is read-only with respect to your Claude Code directory

Perch never writes to, moves, or deletes anything under `~/.claude` (or wherever
`CLAUDE_CONFIG_DIR` points). It reads transcripts and session records; that is
all.

Perch does write to its *own* data directory,
`~/Library/Application Support/Perch/` — the index, `config.toml`, and one-shot
launch scripts. Exactly two files in `perch-core` are permitted to write there
(`db.rs` and `settings/store.rs`), and CI enforces that no other module in
`perch-core`, `perch-ffi` or `perch-mcp` contains a filesystem-writing call.

### 2. No network requests. None.

There is no HTTP client in the dependency graph, no raw socket in the source,
and no telemetry, analytics or crash reporting anywhere. CI fails the build if
an HTTP or telemetry crate appears in any manifest *or* becomes reachable in the
resolved dependency graph, and if any Rust or Swift source names a socket API or
an absolute `http(s)://` URL.

If you have a feature that genuinely needs the network, open an issue first. It
is not an automatic no, but it is a change to the project's central promise and
it will not arrive as a surprise in a pull request.

### 3. No new dependencies

Cargo or SwiftPM. The dependency list is short on purpose: an app that reads
your transcripts should be auditable by a suspicious person in an afternoon. If
you believe a dependency is genuinely necessary, open an issue and make the case
before writing the code.

### 4. Rust owns everything except drawing

Every string the UI displays is produced in `perch-core`. Swift never formats a
number, a duration or a currency, never pluralises a noun, and never compares a
value against a sentinel to decide what to show.

```swift
// No.
Text("\(count) sessions")
Text(cost, format: .currency(code: "USD"))

// Yes. Rust already decided what this reads like.
Text(row.statsLine)
```

Chart *values* cross the FFI boundary as numbers; their *labels* cross as
finished strings. CI fails the build if a Swift `Text` formats a number.

The reason is portability: a later Linux or Windows shell should be able to reuse
the view-model wholesale instead of reimplementing every caption, and every
caption reimplemented is a caption that can disagree.

### 5. An em dash means "not known". A blank string means "the user hid this".

They are different states and must never be conflated. Neither is ever rendered
as a fabricated zero — if a model has no price, the cost is `null` and the model
is named in `unpriced_models`, because a confident `$0.00` is a lie that looks
like data.

### 6. No transcript contents, anywhere

No message text, no code, no file contents from a transcript may enter a
view-model, a notification, an MCP response or a log line. Perch reads
transcripts to count tokens and detect state. It does not repeat what they say.

### 7. Generated output is never committed

`apps/macos/Perch/Sources/PerchFFI/` and `apps/macos/Perch/Frameworks/` are
produced by `scripts/build-xcframework.sh` and are git-ignored. Always
regenerate them; a checked-in copy that drifts from the Rust it binds to is a
classic and very confusing source of FFI bugs.

Please `git add` with explicit paths rather than `git add -A`, so generated
output cannot arrive by accident.

## Getting set up

Requires macOS 15+, [Rust](https://rustup.rs) and Swift 6.

```bash
scripts/build-xcframework.sh        # builds perch-ffi, emits the xcframework + Swift bindings
cd apps/macos/Perch && make bundle  # builds and bundles build/Perch.app
open build/Perch.app
```

`apps/macos/Perch` is a SwiftPM executable; there is no Xcode project.

The data layer alone needs only Rust, and is the easier place to start:

```bash
cargo run --release -p perch-cli -- index
cargo run --release -p perch-cli -- projects
```

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All of it should pass, every time. If something fails intermittently, that is a
bug worth reporting rather than something to re-run until it goes green.

## Tests

New behaviour comes with a test. Test names are sentences describing the
behaviour, not the function under test:

```rust
#[test]
fn an_unknown_terminal_falls_back_to_the_one_macos_always_has() { … }
```

Where a guard protects an invariant, it gets a **positive control** — a test
proving the guard still detects a known offender. A guard that has silently
stopped checking anything is worse than no guard, because it is trusted. The CI
guards in `.github/workflows/ci.yml` all self-test this way; please keep that up
if you touch them.

## Reporting a bug

The Settings window's **Diagnostics** pane answers most "why isn't my session
showing up?" questions directly, and names the resolved paths and index counts.
Including what it says will usually save a round trip.

Please don't paste transcript contents into an issue. Perch goes to some trouble
not to read them; the bug tracker shouldn't undo that.
