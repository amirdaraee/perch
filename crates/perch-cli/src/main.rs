use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use perch_core::{config, db, index, pricing, query};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "perch", about = "Debug CLI for the Perch data layer")]
struct Cli {
    /// Override the Claude Code config directory.
    #[arg(long)]
    config_dir: Option<PathBuf>,

    /// Path to the index database. Defaults to a file beside the config dir.
    #[arg(long)]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan transcripts and update the index.
    Index,
    /// List projects with token totals.
    Projects,
    /// Show usage over trailing windows.
    Usage,
    /// Break usage down by model.
    Models,
    /// List live Claude Code sessions.
    Sessions,
}

fn human_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn human_cost(c: f64) -> String {
    format!("${c:.2}")
}

fn human_elapsed(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

/// A non-positive `since` means the record carried no timestamp at all (e.g.
/// a just-started session with no `statusUpdatedAt` yet) — that is "no data",
/// not "zero elapsed", so it must not be handed to `human_elapsed` as a
/// duration.
fn elapsed_or_dash(now: i64, since: i64) -> String {
    if since <= 0 {
        "—".to_string()
    } else {
        human_elapsed(now - since)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let config_dir = cli
        .config_dir
        .or_else(config::config_dir)
        .context("could not locate a Claude Code config directory; set CLAUDE_CONFIG_DIR")?;

    let db_path = cli.db.unwrap_or_else(|| PathBuf::from("perch-index.db"));
    let database = db::open(&db_path)?;
    pricing::seed_default_prices(&database)?;

    match cli.command {
        Command::Index => {
            let root = config::projects_dir(&config_dir);
            let started = std::time::Instant::now();
            let stats = index::index_all(&database, &root)?;
            println!(
                "indexed {} projects, {} sessions, {} new turns, {} new bytes, {} lines skipped, {} sessions skipped in {:.2}s",
                stats.projects,
                stats.sessions,
                stats.new_turns,
                stats.bytes_read,
                stats.lines_skipped,
                stats.sessions_skipped,
                started.elapsed().as_secs_f64()
            );
        }
        Command::Projects => {
            let mut rows = query::project_summaries(&database)?;
            rows.sort_by_key(|a| std::cmp::Reverse(a.usage.total_tokens()));
            println!(
                "{:<52} {:>4} {:>9} {:>9} {:>10}",
                "PROJECT", "SESS", "TOKENS", "CACHE-RD", "EST COST"
            );
            for r in rows {
                let name = r
                    .display_name
                    .clone()
                    .unwrap_or_else(|| r.real_path.clone());
                let flag = if r.path_is_guess {
                    " (guessed path)"
                } else {
                    ""
                };
                let child = if r.parent_project_id.is_some() {
                    "  ↳ "
                } else {
                    ""
                };
                println!(
                    "{child}{:<52} {:>4} {:>9} {:>9} {:>10}{flag}",
                    truncate(&name, 50),
                    r.sessions,
                    human_tokens(r.usage.total_tokens()),
                    human_tokens(r.usage.cache_read),
                    human_cost(r.cost_usd),
                );
            }
        }
        Command::Usage => {
            let now = chrono::Utc::now().timestamp_millis();
            let five_hours = now - 5 * 60 * 60 * 1000;
            let week = now - 7 * 24 * 60 * 60 * 1000;

            for (label, since) in [
                ("last 5 hours", five_hours),
                ("last 7 days", week),
                ("all time", 0),
            ] {
                let (u, cost) = query::usage_since(&database, since)?;
                println!(
                    "{label:<14} total {:>9}  in {:>8}  out {:>8}  cache-rd {:>9}  cache-wr {:>8}  {:>9}",
                    human_tokens(u.total_tokens()),
                    human_tokens(u.input),
                    human_tokens(u.output),
                    human_tokens(u.cache_read),
                    human_tokens(u.cache_write_total()),
                    human_cost(cost),
                );
            }
        }
        Command::Models => {
            let mut rows = query::usage_by_model(&database)?;
            rows.sort_by_key(|a| std::cmp::Reverse(a.1.total_tokens()));
            println!("{:<34} {:>9} {:>10}", "MODEL", "TOKENS", "EST COST");
            for (model, u, cost) in rows {
                println!(
                    "{:<34} {:>9} {:>10}",
                    truncate(&model, 32),
                    human_tokens(u.total_tokens()),
                    human_cost(cost)
                );
            }
        }
        Command::Sessions => {
            use perch_core::live::{live_sessions, SessionStatus};
            use perch_core::platform::RealProcessProbe;

            let probe = RealProcessProbe;
            let sessions = live_sessions(&config::sessions_dir(&config_dir), &probe);
            let now = chrono::Utc::now().timestamp_millis();

            if sessions.is_empty() {
                println!("no live sessions");
            }
            println!(
                "{:<40} {:<10} {:<24} {:>6}",
                "SESSION", "KIND", "STATUS", "FOR"
            );
            for s in sessions {
                let (label, since) = match &s.status {
                    SessionStatus::Waiting { reason, since_ms } => (
                        format!("waiting · {}", reason.as_deref().unwrap_or("unknown")),
                        *since_ms,
                    ),
                    SessionStatus::Idle => ("idle".to_string(), s.status_updated_at),
                    SessionStatus::Working => ("working".to_string(), s.status_updated_at),
                    _ => ("working".to_string(), s.status_updated_at),
                };
                println!(
                    "{:<40} {:<10} {:<24} {:>6}",
                    truncate(&s.name, 38),
                    s.kind,
                    truncate(&label, 22),
                    elapsed_or_dash(now, since),
                );
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(s.chars().count() - (max - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_large_token_counts_compactly() {
        assert_eq!(human_tokens(0), "0");
        assert_eq!(human_tokens(999), "999");
        assert_eq!(human_tokens(1_500), "1.5k");
        assert_eq!(human_tokens(24_221), "24.2k");
        assert_eq!(human_tokens(4_100_000), "4.1M");
    }

    #[test]
    fn formats_cost_with_two_decimals() {
        assert_eq!(human_cost(0.0), "$0.00");
        assert_eq!(human_cost(8.204), "$8.20");
        assert_eq!(human_cost(38.0), "$38.00");
    }

    #[test]
    fn formats_elapsed_durations_compactly() {
        assert_eq!(human_elapsed(0), "0s");
        assert_eq!(human_elapsed(45_000), "45s");
        assert_eq!(human_elapsed(90_000), "1m");
        assert_eq!(human_elapsed(3_600_000), "1h");
        assert_eq!(human_elapsed(115_200_000), "32h");
    }

    #[test]
    fn a_missing_timestamp_is_a_dash_not_a_giant_duration() {
        let now = 1_000_000_000;
        assert_eq!(elapsed_or_dash(now, 0), "—");
        assert_eq!(elapsed_or_dash(now, -5), "—");
    }

    #[test]
    fn a_real_timestamp_formats_the_same_as_human_elapsed() {
        let now = 1_000_000_000;
        let since = now - 45_000;
        assert_eq!(elapsed_or_dash(now, since), human_elapsed(now - since));
    }
}
