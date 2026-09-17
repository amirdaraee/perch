#!/usr/bin/env python3
"""Build a throwaway Claude Code directory for screenshots and demos.

Perch's screenshots would otherwise show real projects, real branches and real
spend. This writes an invented one instead — plausible transcripts for a handful
of made-up projects — and prints the environment that points Perch at it.

Nothing here touches your real data. `PERCH_DATA_DIR` relocates Perch's index
and settings; `CLAUDE_CONFIG_DIR` relocates what it reads. Both are read by
perch-core directly (settings/store.rs, config.rs), so no code changes are
needed and your own index is never opened.

    python3 scripts/demo-index.py /tmp/perch-demo
    eval "$(python3 scripts/demo-index.py /tmp/perch-demo --env)"
    /path/to/Perch.app/Contents/MacOS/Perch

Run with --live to also start fake `claude` processes so the live-session rows
have something to show. They are `sleep` processes and exit on their own.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import shutil
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

# Only models with a seeded price, so the screenshots show costs rather than
# "unpriced". See crates/perch-core/src/pricing.rs.
MODELS = [
    ("claude-opus-5", 0.30),
    ("claude-sonnet-5", 0.55),
    ("claude-haiku-4-5-20251001", 0.15),
]

# name, branch, README description, days active, weight (relative token volume)
PROJECTS = [
    ("hilltop", "main",
     "A static site generator that does nothing clever. Markdown in, HTML out, "
     "no plugins, no config file, no opinions about your CSS.", 14, 1.00),
    ("notes-api", "feat/sharing",
     "REST API for a notes application. Go and Postgres, with a migration tool "
     "that refuses to run backwards.", 13, 0.72),
    ("birdsong", "main",
     "Audio classifier for garden bird recordings. Trains on your own labelled "
     "clips rather than a general model.", 11, 0.55),
    ("ledger-cli", "refactor/accounts",
     "Double-entry bookkeeping from the command line. Plain-text ledger files, "
     "no database, no subscription.", 9, 0.40),
    ("atlas-web", "main",
     "Marketing site and documentation for Atlas. Astro, with the docs "
     "generated from the API schema.", 6, 0.28),
    ("dotfiles", "main",
     "Shell, editor and terminal configuration. Bootstraps a new machine in one "
     "command and is careful never to overwrite anything it did not write.",
     4, 0.12),
]

SESSION_TITLES = [
    "Fix the pagination cursor on the search endpoint",
    "Rewrite the config loader to fail loudly",
    "Add a golden test for the markdown renderer",
    "Track down the flaky timezone test",
    "Port the CLI to the new argument parser",
    "Cache the schema between runs",
    "Split the migration runner out of main",
    "Make the error messages say what to do next",
    "Drop the last of the global state",
    "Tighten the retry backoff",
]


def rfc3339(when: datetime) -> str:
    return when.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"


def assistant_line(when: datetime, cwd: str, branch: str, model: str,
                   rng: random.Random) -> str:
    """One assistant turn in the shape Claude Code writes it.

    A turn is counted only when both `message.usage` and `timestamp` are
    present, and the timestamp must be RFC 3339 (see perch-core's transcript.rs
    and scan.rs).
    """
    inp = rng.randint(2, 40)
    out = rng.randint(120, 2400)
    cache_write = rng.randint(1_000, 30_000)
    cache_read = rng.randint(5_000, 60_000)
    return json.dumps({
        "type": "assistant",
        "cwd": cwd,
        "gitBranch": branch,
        "version": "2.1.251",
        "timestamp": rfc3339(when),
        "message": {
            "model": model,
            "usage": {
                "input_tokens": inp,
                "output_tokens": out,
                "cache_creation_input_tokens": cache_write,
                "cache_read_input_tokens": cache_read,
                "output_tokens_details": {"thinking_tokens": out // 4},
                "cache_creation": {
                    "ephemeral_5m_input_tokens": cache_write,
                    "ephemeral_1h_input_tokens": 0,
                },
            },
        },
    })


def user_line(when: datetime, cwd: str, branch: str) -> str:
    """A user turn. No usage, so it adds a message but no tokens."""
    return json.dumps({
        "type": "user",
        "cwd": cwd,
        "gitBranch": branch,
        "version": "2.1.251",
        "timestamp": rfc3339(when),
        "message": {"role": "user"},
    })


def build(root: Path, code: Path, seed: int) -> list[tuple[str, str, str]]:
    """Write the demo tree. Returns (name, cwd, branch) per project."""
    rng = random.Random(seed)
    claude = root / "claude"
    for d in (claude / "projects", claude / "sessions", code, root / "perch-data"):
        d.mkdir(parents=True, exist_ok=True)

    now = datetime.now(timezone.utc)
    built = []

    for name, branch, description, active_days, weight in PROJECTS:
        cwd = code / name
        cwd.mkdir(parents=True, exist_ok=True)
        # Perch reads the project's description from the README in its working
        # directory (perch-core/src/readme.rs), so the demo needs real files.
        (cwd / "README.md").write_text(f"# {name}\n\n{description}\n")

        slug = str(cwd).replace("/", "-")
        project_dir = claude / "projects" / slug
        project_dir.mkdir(parents=True, exist_ok=True)

        for day in range(active_days):
            # Not every day has a session: a sparkline of identical bars looks
            # synthetic, which is exactly what it would be.
            if day > 0 and rng.random() < 0.25:
                continue
            sessions_today = 1 if rng.random() < 0.7 else 2
            for s in range(sessions_today):
                # Offset backwards from now rather than picking a clock time on
                # that day: a random hour on day 0 lands in the future, and the
                # UI renders a session that has not happened yet as "ended 0s
                # ago", which is the one thing in a screenshot that shouts
                # "synthetic".
                start = now - timedelta(days=day,
                                        hours=rng.uniform(1.5, 11.0))
                session_id = f"{name}-{day:02d}-{s}"
                lines = [json.dumps({
                    "type": "ai-title",
                    "aiTitle": rng.choice(SESSION_TITLES),
                    "cwd": str(cwd),
                    "gitBranch": branch,
                    "timestamp": rfc3339(start),
                })]
                turns = max(2, int(rng.randint(4, 22) * weight))
                when = start
                for t in range(turns):
                    when += timedelta(minutes=rng.randint(1, 9))
                    model = rng.choices([m for m, _ in MODELS],
                                        [w for _, w in MODELS])[0]
                    lines.append(user_line(when, str(cwd), branch))
                    when += timedelta(seconds=rng.randint(20, 400))
                    lines.append(assistant_line(when, str(cwd), branch, model, rng))
                (project_dir / f"{session_id}.jsonl").write_text(
                    "\n".join(lines) + "\n")

        built.append((name, str(cwd), branch))

    return built


def start_live(root: Path,
               projects: list[tuple[str, str, str]]) -> list[subprocess.Popen]:
    """Start fake `claude` processes and write matching session records.

    Perch only accepts a live record when the pid is alive AND
    `ps -o comm=` on it has the basename `claude` (perch-core's live.rs and
    platform/macos.rs). So the demo needs real processes actually named
    `claude` — a copy of `sleep` is enough.
    """
    fake = root / "bin" / "claude"
    fake.parent.mkdir(parents=True, exist_ok=True)
    # copy, not copy2: /bin/sleep carries a restricted system flag that
    # copystat tries to reproduce and cannot, which fails with EPERM.
    shutil.copy("/bin/sleep", fake)
    fake.chmod(0o755)
    # A copied Apple binary no longer validates against the platform trust
    # cache, and macOS SIGKILLs it on launch (exit 137, no error message).
    # Ad-hoc signing the copy makes it a valid binary in its own right.
    subprocess.run(["codesign", "--force", "--sign", "-", str(fake)],
                   check=True, capture_output=True)

    now_ms = int(datetime.now(timezone.utc).timestamp() * 1000)
    states = [
        ("waiting", "permission to run a command", "interactive", 0),
        ("busy", None, "interactive", 1),
        ("busy", None, "bg", 2),
        ("idle", None, "interactive", 3),
    ]

    pids = []
    sessions = root / "claude" / "sessions"
    # Stale records from a previous run would point at pids that are dead, or
    # worse, recycled onto some unrelated process.
    for old in sessions.glob("*.json"):
        old.unlink()

    for status, waiting_for, kind, idx in states:
        name, cwd, _ = projects[idx % len(projects)]
        proc = subprocess.Popen([str(fake), "3600"],
                                stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL)
        pids.append(proc)
        record = {
            "pid": proc.pid,
            "sessionId": f"live-{name}-{idx}",
            "cwd": cwd,
            "name": SESSION_TITLES[idx],
            "kind": kind,
            "status": status,
            "version": "2.1.251",
            "startedAt": now_ms - (idx + 1) * 37 * 60 * 1000,
            "statusUpdatedAt": now_ms - (idx + 1) * 11 * 60 * 1000,
        }
        if waiting_for:
            record["waitingFor"] = waiting_for
        (sessions / f"{proc.pid}.json").write_text(json.dumps(record))

    return pids


def hold(procs: list[subprocess.Popen], sessions: Path) -> None:
    """Stay alive so the fake sessions do.

    These have to be ordinary children of a living process: a detached one
    outlives the shell that started it but is no longer waitable, and a plain
    child is reaped the moment its parent shell exits. So the script itself is
    what keeps them up, and tidies up on the way out.
    """
    print()
    print("Holding the fake sessions open. Ctrl-C when you're done.")
    try:
        for proc in procs:
            proc.wait()
    except KeyboardInterrupt:
        pass
    finally:
        for proc in procs:
            if proc.poll() is None:
                proc.terminate()
        for record in sessions.glob("*.json"):
            record.unlink()
        print("\ntidied up: fake processes stopped, live records removed")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("root", nargs="?", default="/tmp/perch-demo",
                    help="where to build the demo tree (default: /tmp/perch-demo)")
    ap.add_argument("--env", action="store_true",
                    help="print only the export lines, for eval")
    ap.add_argument("--live", action="store_true",
                    help="also start fake `claude` processes for live rows")
    ap.add_argument("--code-root", default=None,
                    help="where the demo project folders live. Perch shows a "
                         "project's full working directory, so a long or "
                         "personal path ends up in the screenshot; "
                         "/Users/Shared/code reads naturally and names nobody. "
                         "Default: <root>/code")
    ap.add_argument("--seed", type=int, default=7,
                    help="random seed, so a rebuild reproduces the same data")
    ap.add_argument("--keep", action="store_true",
                    help="add to an existing tree instead of wiping it")
    ap.add_argument("--wait", action="store_true",
                    help="with --live, stay running so the fake processes stay "
                         "alive; Ctrl-C tidies them up")
    args = ap.parse_args()

    root = Path(args.root).expanduser().resolve()
    exports = (f'export PERCH_DATA_DIR="{root}/perch-data"\n'
               f'export CLAUDE_CONFIG_DIR="{root}/claude"')

    if args.env:
        print(exports)
        return 0

    if root.exists() and not args.keep:
        shutil.rmtree(root)

    code = (Path(args.code_root).expanduser().resolve()
            if args.code_root else root / "code")
    projects = build(root, code, args.seed)

    procs = start_live(root, projects) if args.live else []

    n_sessions = sum(1 for _ in (root / "claude" / "projects").rglob("*.jsonl"))
    print(f"demo tree:      {root}")
    print(f"project dirs:   {code}")
    print(f"projects:       {len(projects)}")
    print(f"transcripts:    {n_sessions}")
    if procs:
        pids = " ".join(str(p.pid) for p in procs)
        print(f"live processes: {pids}")
    print()
    print("Point Perch at it — note that `open -a` does NOT pass environment")
    print("variables to the app, so run the executable inside the bundle:")
    print()
    print(exports)
    print(f'  apps/macos/Perch/build/Perch.app/Contents/MacOS/Perch')

    if args.wait and procs:
        hold(procs, root / "claude" / "sessions")

    return 0


if __name__ == "__main__":
    sys.exit(main())
