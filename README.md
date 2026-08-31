# Perch

A dashboard for Claude Code — track your projects, live sessions, and token usage.

> **Status:** early development. The data layer works; the app UI is in progress.

Not affiliated with or endorsed by Anthropic.

## What it does

Perch reads the data Claude Code already writes to disk and answers three questions:

- **Which session is stuck waiting on me, and which terminal owns it?**
- **What was each project doing, and where did I leave off?**
- **How much of my rate-limit window is left, and where did the tokens go?**

## Privacy

Your transcripts contain source code, pasted secrets, and client names. So:

- **No telemetry, no analytics, no crash reporting.** Ever.
- **Read-only.** Perch never writes to, moves, or deletes anything in your Claude Code directory.
- **One outbound host,** and only in the app itself: `anthropic.com`, to read your own rate-limit
  status. The data layer in this repository makes no network requests at all, and CI fails the
  build if an HTTP client is added to it.

## Try the data layer

Requires [Rust](https://rustup.rs).

```bash
cargo run --release -p perch-cli -- index
cargo run --release -p perch-cli -- projects
cargo run --release -p perch-cli -- usage
cargo run --release -p perch-cli -- models
```

Perch finds your Claude Code data at `$CLAUDE_CONFIG_DIR`, then `$XDG_CONFIG_HOME/claude`,
then `~/.claude`. Override it with `--config-dir`.

## How it works

Transcripts are append-only, so Perch stores a byte offset per file and re-parses only new
bytes on each pass — a full re-index of 120 MB happens once, and every pass after that costs
microseconds per active session.

Project paths come from the `cwd` field inside transcripts, never from decoding the directory
name. That encoding flattens both `/` and `.` into `-`, so `-Users-me-projects-foo-github-io`
is ambiguous and decoding it picks the wrong directory.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## License

MIT
