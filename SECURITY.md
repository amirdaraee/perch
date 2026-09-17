# Security

## Reporting a vulnerability

Please report privately, via **[GitHub's private vulnerability
reporting](https://github.com/amirdaraee/perch/security/advisories/new)** — the
Security tab of this repository. Please don't open a public issue for something
exploitable.

I'll acknowledge the report as quickly as I reasonably can, and I'll tell you
plainly if I don't intend to fix something rather than leaving you waiting.

Please do not include transcript contents in a report. If reproducing an issue
seems to require them, say so and we'll work out a redacted way to do it.

## What the attack surface actually is

Perch is a local, read-only macOS app. That shapes what a vulnerability in it
can look like:

- **There is no network code.** No HTTP client is in the dependency graph, no
  raw socket appears in the source, and CI fails the build if either changes.
  There is no server, no listening port, and no remote endpoint to attack.
- **There is no authentication, and no secrets are stored.** Perch holds no
  tokens or credentials of any kind.
- **Perch does not write to your Claude Code directory**, so it cannot corrupt
  the data it reads.

What remains, and what I would most want to hear about:

- **Transcript parsing.** Perch parses `.jsonl` files it did not write. A
  malformed or hostile transcript causing a panic, unbounded memory growth, or
  anything worse is a real bug.
- **Command composition.** Resume and "new session" compose a shell command from
  a project path and a session id and hand it to a terminal. It is built in
  `perch-core::actions`, which POSIX-single-quotes every embedded value. A way
  to break out of that quoting is the most serious bug I can think of in this
  codebase — a project path is attacker-influenced if you ever open someone
  else's repository.
- **The MCP server.** `perch-mcp` opens the index read-only and answers on
  stdio. Anything that makes it write, reach outside the index, or emit
  transcript contents is a vulnerability.
- **Data leaking into somewhere it shouldn't.** Perch is designed so that no
  message text reaches a view-model, a notification, an MCP response or a log
  line. A path where it does is a bug worth reporting even if it needs no
  attacker to trigger.

## Supported versions

Perch is pre-1.0. Fixes go onto `main` and into the next release; there are no
maintained release branches.
