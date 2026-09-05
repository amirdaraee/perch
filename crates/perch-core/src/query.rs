//! Read-side aggregation.

use crate::db::Db;
use crate::model::TurnUsage;
use crate::pricing::{cost_usd, price_for};
use anyhow::Result;
use rusqlite::params;

#[derive(Debug, Clone)]
pub struct ProjectSummary {
    pub id: i64,
    pub slug: String,
    pub real_path: String,
    pub display_name: Option<String>,
    pub parent_project_id: Option<i64>,
    pub path_is_guess: bool,
    pub sessions: i64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
    pub last_activity_at: Option<i64>,
}

/// Sum usage per (scope, model) so each model's rate applies to its own tokens.
fn usage_rows(
    db: &Db,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<(String, TurnUsage)>> {
    let mut stmt = db.conn().prepare(sql)?;
    let rows = stmt.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            TurnUsage {
                input: r.get::<_, i64>(1)? as u64,
                output: r.get::<_, i64>(2)? as u64,
                cache_read: r.get::<_, i64>(3)? as u64,
                cache_write_5m: r.get::<_, i64>(4)? as u64,
                cache_write_1h: r.get::<_, i64>(5)? as u64,
                thinking: r.get::<_, i64>(6)? as u64,
            },
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

const SUMS: &str = "COALESCE(SUM(input),0), COALESCE(SUM(output),0), COALESCE(SUM(cache_read),0), \
                    COALESCE(SUM(cache_write_5m),0), COALESCE(SUM(cache_write_1h),0), \
                    COALESCE(SUM(thinking),0)";

/// Unknown models contribute tokens but no cost, per spec §8.
fn cost_of(db: &Db, model: &str, usage: &TurnUsage) -> Result<f64> {
    Ok(match price_for(db, model)? {
        Some(p) => cost_usd(&p, usage),
        None => 0.0,
    })
}

/// Reduce per-model usage rows into a total: tokens summed across models, and
/// cost summed after pricing each model's tokens at that model's own rate.
fn priced_usage(db: &Db, per_model: &[(String, TurnUsage)]) -> Result<(TurnUsage, f64)> {
    let mut usage = TurnUsage::default();
    let mut cost = 0.0;
    for (model, u) in per_model {
        usage = usage.plus(u);
        cost += cost_of(db, model, u)?;
    }
    Ok((usage, cost))
}

pub fn project_summaries(db: &Db) -> Result<Vec<ProjectSummary>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, slug, real_path, display_name, parent_project_id, path_is_guess
         FROM projects ORDER BY real_path",
    )?;
    let bases = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, i64>(5)? != 0,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (id, slug, real_path, display_name, parent_project_id, path_is_guess) in bases {
        let sessions: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM sessions WHERE project_id = ?1",
            params![id],
            |r| r.get(0),
        )?;

        let per_model = usage_rows(
            db,
            &format!(
                "SELECT model, {SUMS} FROM turns
                 WHERE session_id IN (SELECT id FROM sessions WHERE project_id = ?1)
                 GROUP BY model"
            ),
            &[&id],
        )?;

        let (usage, cost) = priced_usage(db, &per_model)?;

        let last_activity_at: Option<i64> = db.conn().query_row(
            "SELECT MAX(ts) FROM turns
             WHERE session_id IN (SELECT id FROM sessions WHERE project_id = ?1)",
            params![id],
            |r| r.get(0),
        )?;

        out.push(ProjectSummary {
            id,
            slug,
            real_path,
            display_name,
            parent_project_id,
            path_is_guess,
            sessions,
            usage,
            cost_usd: cost,
            last_activity_at,
        });
    }
    Ok(out)
}

/// Serves both clocks: a trailing 5-hour window and a trailing week differ only
/// in `since_ms`. Pass 0 for all time.
pub fn usage_since(db: &Db, since_ms: i64) -> Result<(TurnUsage, f64)> {
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns WHERE ts >= ?1 GROUP BY model"),
        &[&since_ms],
    )?;
    priced_usage(db, &per_model)
}

/// The timestamp of the earliest turn at or after `since_ms`, or `None` if
/// there are none. Used to measure how much of a trailing window Perch has
/// actually observed turns for — e.g. the usage view's burn-rate projection,
/// which anchors elapsed time to this rather than to an assumed window
/// boundary it cannot know.
pub fn oldest_turn_since(db: &Db, since_ms: i64) -> Result<Option<i64>> {
    let ts: Option<i64> = db.conn().query_row(
        "SELECT MIN(ts) FROM turns WHERE ts >= ?1",
        params![since_ms],
        |r| r.get(0),
    )?;
    Ok(ts)
}

pub fn usage_by_model(db: &Db) -> Result<Vec<(String, TurnUsage, f64)>> {
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns GROUP BY model"),
        &[],
    )?;
    let mut out = Vec::new();
    for (model, u) in per_model {
        let c = cost_of(db, &model, &u)?;
        out.push((model, u, c));
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct RecentSession {
    pub id: String,
    pub cwd: Option<String>,
    pub last_activity_at: i64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
}

/// Tokens and estimated cost for one session, priced per model and summed.
/// An unknown session is simply zero — it is not an error to ask.
pub fn session_usage(db: &Db, session_id: &str) -> Result<(TurnUsage, f64)> {
    let args: [&dyn rusqlite::ToSql; 1] = [&session_id];
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns WHERE session_id = ?1 GROUP BY model"),
        &args,
    )?;
    priced_usage(db, &per_model)
}

/// The most recently active sessions that are NOT currently live, newest first.
pub fn recent_sessions(
    db: &Db,
    exclude_ids: &[String],
    limit: usize,
) -> Result<Vec<RecentSession>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = db.conn().prepare(
        "SELECT id, cwd, last_activity_at FROM sessions
         WHERE last_activity_at IS NOT NULL
         ORDER BY last_activity_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, cwd, last) = row?;
        if exclude_ids.iter().any(|x| x == &id) {
            continue;
        }
        let (usage, cost_usd) = session_usage(db, &id)?;
        out.push(RecentSession {
            id,
            cwd,
            last_activity_at: last,
            usage,
            cost_usd,
        });
        if out.len() == limit {
            break;
        }
    }
    Ok(out)
}

pub const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone)]
pub struct HistorySession {
    pub id: String,
    pub started_at: Option<i64>,
    pub last_activity_at: Option<i64>,
    pub git_branch: Option<String>,
    pub message_count: u64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
}

/// Every session ever recorded for a project, newest first. Sessions with no
/// recorded activity sort last rather than being dropped — they exist, and the
/// window's job is to show what exists.
pub fn session_history(db: &Db, project_id: i64) -> Result<Vec<HistorySession>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, started_at, last_activity_at, git_branch, message_count
         FROM sessions WHERE project_id = ?1
         -- `last_activity_at IS NULL` is 0 for dated rows and 1 for NULL ones,
         -- so ordering by it first pushes NULLs after every dated row, which
         -- plain `ORDER BY last_activity_at DESC` would instead put first.
         ORDER BY last_activity_at IS NULL, last_activity_at DESC",
    )?;
    let rows = stmt.query_map(params![project_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)? as u64,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, started_at, last_activity_at, git_branch, message_count) = row?;
        let (usage, cost_usd) = session_usage(db, &id)?;
        out.push(HistorySession {
            id,
            started_at,
            last_activity_at,
            git_branch,
            message_count,
            usage,
            cost_usd,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct DayUsage {
    pub day_start_ms: i64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
}

fn day_start(ts_ms: i64) -> i64 {
    ts_ms - ts_ms.rem_euclid(DAY_MS)
}

/// Shared by `daily_usage` and `daily_usage_for_project`: everything except
/// which turns are in scope (all of them, or one project's) is identical.
fn daily_usage_grouped(
    db: &Db,
    days: usize,
    now_ms: i64,
    project_filter: Option<i64>,
) -> Result<Vec<DayUsage>> {
    let last = day_start(now_ms);
    let first = last - (days as i64 - 1).max(0) * DAY_MS;
    // Exclusive upper bound: one day past `last`, so turns newer than the
    // requested window are never fetched, priced, or grouped in the first
    // place (rather than being computed and then discarded).
    let upper = last + DAY_MS;

    let mut per_day: std::collections::HashMap<i64, (TurnUsage, f64)> =
        std::collections::HashMap::new();
    let scope = match project_filter {
        Some(_) => "AND session_id IN (SELECT id FROM sessions WHERE project_id = ?3)",
        None => "",
    };
    let sql = format!(
        // `ts % D` in SQLite is a C-style remainder, already in [0, D) for
        // non-negative `ts`; the outer `(+ D) % D` only matters for negative
        // `ts` (pre-1970), pulling a negative remainder back into [0, D) so
        // this agrees with Rust's `ts.rem_euclid(D)` in `day_start` above.
        "SELECT (ts - (ts % {DAY_MS} + {DAY_MS}) % {DAY_MS}) AS day, model, {SUMS}
         FROM turns WHERE ts >= ?1 AND ts < ?2 {scope} GROUP BY day, model"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let rows = if let Some(project_id) = project_filter {
        stmt.query_map(params![first, upper, project_id], day_model_usage_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        stmt.query_map(params![first, upper], day_model_usage_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (day, model, u) in rows {
        let cost = cost_of(db, &model, &u)?;
        let slot = per_day.entry(day).or_insert((TurnUsage::default(), 0.0));
        slot.0 = slot.0.plus(&u);
        slot.1 += cost;
    }

    Ok((0..days)
        .map(|i| {
            let day_start_ms = first + i as i64 * DAY_MS;
            let (usage, cost_usd) = per_day.get(&day_start_ms).cloned().unwrap_or_default();
            DayUsage {
                day_start_ms,
                usage,
                cost_usd,
            }
        })
        .collect())
}

fn day_model_usage_row(r: &rusqlite::Row) -> rusqlite::Result<(i64, String, TurnUsage)> {
    Ok((
        r.get::<_, i64>(0)?,
        r.get::<_, String>(1)?,
        TurnUsage {
            input: r.get::<_, i64>(2)? as u64,
            output: r.get::<_, i64>(3)? as u64,
            cache_read: r.get::<_, i64>(4)? as u64,
            cache_write_5m: r.get::<_, i64>(5)? as u64,
            cache_write_1h: r.get::<_, i64>(6)? as u64,
            thinking: r.get::<_, i64>(7)? as u64,
        },
    ))
}

/// `days` consecutive days ending with the one containing `now_ms`, oldest first.
/// Quiet days are present and zeroed: a bar chart with gaps silently lies about
/// the shape of the week.
pub fn daily_usage(db: &Db, days: usize, now_ms: i64) -> Result<Vec<DayUsage>> {
    daily_usage_grouped(db, days, now_ms, None)
}

/// Same shape as `daily_usage`, scoped to one project — the main window's
/// per-project sparkline must not silently show every project's activity on
/// a single project's page.
pub fn daily_usage_for_project(
    db: &Db,
    project_id: i64,
    days: usize,
    now_ms: i64,
) -> Result<Vec<DayUsage>> {
    daily_usage_grouped(db, days, now_ms, Some(project_id))
}

/// Projects ranked by tokens since `since_ms`. The label is the directory name,
/// which is what the user recognises — not the slug and not the whole path.
pub fn top_projects(db: &Db, since_ms: i64, limit: usize) -> Result<Vec<(String, TurnUsage, f64)>> {
    let mut stmt = db.conn().prepare(&format!(
        "SELECT p.id, p.real_path, p.display_name, t.model, {SUMS}
         FROM turns t
         JOIN sessions s ON s.id = t.session_id
         JOIN projects p ON p.id = s.project_id
         WHERE t.ts >= ?1
         GROUP BY p.id, t.model"
    ))?;
    let rows = stmt.query_map(params![since_ms], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            TurnUsage {
                input: r.get::<_, i64>(4)? as u64,
                output: r.get::<_, i64>(5)? as u64,
                cache_read: r.get::<_, i64>(6)? as u64,
                cache_write_5m: r.get::<_, i64>(7)? as u64,
                cache_write_1h: r.get::<_, i64>(8)? as u64,
                thinking: r.get::<_, i64>(9)? as u64,
            },
        ))
    })?;

    // Keyed by project id, not by label: two distinct projects can share a
    // directory basename (nested checkouts named the same leaf directory are
    // ordinary), and folding by label would silently merge their usage.
    let mut totals: std::collections::HashMap<i64, (String, TurnUsage, f64)> =
        std::collections::HashMap::new();
    for row in rows {
        let (project_id, real_path, display_name, model, u) = row?;
        // `real_path`/`display_name` are identical across every row for this
        // `project_id` (the join groups by `p.id, t.model`), so recomputing
        // the label on each row rather than caching it on first insert is
        // cheap and side-steps having to distinguish "not yet set" from "set
        // to an empty string".
        let label = display_name.unwrap_or_else(|| {
            std::path::Path::new(&real_path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(real_path.clone())
        });
        let cost = cost_of(db, &model, &u)?;
        let slot = totals
            .entry(project_id)
            .or_insert_with(|| (label.clone(), TurnUsage::default(), 0.0));
        slot.0 = label;
        slot.1 = slot.1.plus(&u);
        slot.2 += cost;
    }

    let mut out: Vec<(String, TurnUsage, f64)> = totals.into_values().collect();
    out.sort_by(|a, b| {
        b.1.total_tokens()
            .cmp(&a.1.total_tokens())
            .then_with(|| a.0.cmp(&b.0))
    });
    out.truncate(limit);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn};
    use crate::pricing::seed_default_prices;

    fn setup() -> Db {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-one", "/Users/a/one", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            id: "s1".into(),
            project_id: pid,
            file_path: "/tmp/s1.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(1_000),
            last_activity_at: Some(2_000),
            cwd: None,
            git_branch: None,
            cc_version: None,
            message_count: 2,
        })
        .unwrap();
        db.insert_turns(
            "s1",
            &[
                Turn {
                    ts: 1_000,
                    model: "claude-fable-5".into(),
                    usage: TurnUsage {
                        input: 1_000_000,
                        ..Default::default()
                    },
                },
                Turn {
                    ts: 5_000,
                    model: "claude-haiku-4-5-20251001".into(),
                    usage: TurnUsage {
                        output: 1_000_000,
                        ..Default::default()
                    },
                },
            ],
        )
        .unwrap();
        db
    }

    #[test]
    fn summarizes_a_project_with_tokens_and_cost() {
        let db = setup();
        let rows = project_summaries(&db).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.sessions, 1);
        assert_eq!(r.usage.input, 1_000_000);
        assert_eq!(r.usage.output, 1_000_000);
        // fable input 1 MTok @ 15.0/MTok + haiku output 1 MTok @ 5.0/MTok.
        // The two rates must stay different: if they were equal, a regression
        // that pools tokens across models and multiplies by a single rate
        // would still land on the right total. Do not "tidy" this fixture
        // back to two equal-rate models.
        assert!((r.cost_usd - 20.0).abs() < 1e-9);
        assert_eq!(r.last_activity_at, Some(5_000));
    }

    #[test]
    fn usage_since_filters_by_timestamp() {
        let db = setup();
        let (usage, cost) = usage_since(&db, 4_000).unwrap();
        assert_eq!(usage.input, 0);
        assert_eq!(usage.output, 1_000_000);
        assert!((cost - 5.0).abs() < 1e-9);

        // The boundary is inclusive (`ts >= since_ms`): querying exactly at
        // the second turn's timestamp must still include it.
        let (usage_at_boundary, _) = usage_since(&db, 5_000).unwrap();
        assert_eq!(usage_at_boundary.output, 1_000_000);
    }

    #[test]
    fn usage_since_with_zero_returns_everything() {
        let db = setup();
        let (usage, _) = usage_since(&db, 0).unwrap();
        assert_eq!(usage.input, 1_000_000);
        assert_eq!(usage.output, 1_000_000);
    }

    #[test]
    fn breaks_usage_down_by_model() {
        let db = setup();
        let mut rows = usage_by_model(&db).unwrap();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "claude-fable-5");
        assert_eq!(rows[0].1.input, 1_000_000);
    }

    #[test]
    fn unknown_model_is_counted_with_zero_cost() {
        let db = setup();
        db.insert_turns(
            "s1",
            &[Turn {
                ts: 6_000,
                model: "model-from-the-future".into(),
                usage: TurnUsage {
                    input: 2_000_000,
                    ..Default::default()
                },
            }],
        )
        .unwrap();

        let rows = project_summaries(&db).unwrap();
        assert_eq!(
            rows[0].usage.input, 3_000_000,
            "tokens must still be counted"
        );
        assert!(
            (rows[0].cost_usd - 20.0).abs() < 1e-9,
            "unpriced model adds no cost"
        );
    }

    #[test]
    fn a_project_with_no_sessions_still_appears() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        db.upsert_project("-Users-a-empty", "/Users/a/empty", true)
            .unwrap();
        let rows = project_summaries(&db).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sessions, 0);
        assert_eq!(rows[0].usage.total_tokens(), 0);
        assert!(rows[0].path_is_guess);
    }

    fn seed_session(db: &Db, pid: i64, id: &str, cwd: &str, last: i64, input: u64) {
        db.upsert_session(&SessionRecord {
            id: id.into(),
            project_id: pid,
            file_path: format!("/tmp/{id}.jsonl"),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(last - 1000),
            last_activity_at: Some(last),
            cwd: Some(cwd.into()),
            git_branch: None,
            cc_version: None,
            message_count: 1,
        })
        .unwrap();
        db.insert_turns(
            id,
            &[Turn {
                ts: last,
                model: "claude-fable-5".into(),
                usage: TurnUsage {
                    input,
                    ..Default::default()
                },
            }],
        )
        .unwrap();
    }

    #[test]
    fn session_usage_prices_per_model() {
        // claude-fable-5 and claude-opus-5 have byte-identical default rates,
        // so a single-model fixture can't prove per-model pricing is applied
        // (a bug that resolved the wrong model's price would still pass).
        // Use two models with genuinely different input rates instead:
        // fable at $15.0/MTok and sonnet at $3.0/MTok.
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-one", "/Users/a/one", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            id: "s-a".into(),
            project_id: pid,
            file_path: "/tmp/s-a.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(9_000),
            last_activity_at: Some(10_000),
            cwd: Some("/Users/a/one".into()),
            git_branch: None,
            cc_version: None,
            message_count: 2,
        })
        .unwrap();
        db.insert_turns(
            "s-a",
            &[
                Turn {
                    ts: 9_000,
                    model: "claude-fable-5".into(),
                    usage: TurnUsage {
                        input: 2_000_000,
                        ..Default::default()
                    },
                },
                Turn {
                    ts: 10_000,
                    model: "claude-sonnet-5".into(),
                    usage: TurnUsage {
                        input: 1_000_000,
                        ..Default::default()
                    },
                },
            ],
        )
        .unwrap();
        let (u, cost) = session_usage(&db, "s-a").unwrap();
        assert_eq!(u.input, 3_000_000);
        // Reachable only if fable's 2 MTok is priced at $15.0/MTok ($30.00)
        // and sonnet's 1 MTok is priced at its own $3.0/MTok ($3.00), summed.
        // Pooling both under either rate would give a different total.
        assert!(
            (cost - 33.0).abs() < 1e-9,
            "fable 2 MTok @ 15.0 + sonnet 1 MTok @ 3.0 = 33.00"
        );
    }

    #[test]
    fn session_usage_for_unknown_session_is_zero_not_error() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let (u, cost) = session_usage(&db, "nope").unwrap();
        assert_eq!(u.total_tokens(), 0);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn recent_sessions_are_newest_first_and_skip_live_ones() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-one", "/Users/a/one", false)
            .unwrap();
        seed_session(&db, pid, "old", "/Users/a/one", 1_000, 1);
        seed_session(&db, pid, "mid", "/Users/a/one", 2_000, 1);
        seed_session(&db, pid, "new", "/Users/a/one", 3_000, 1);
        seed_session(&db, pid, "live", "/Users/a/one", 4_000, 1);

        let rows = recent_sessions(&db, &["live".to_string()], 2).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["new", "mid"],
            "newest first, live excluded, limited to 2"
        );
        assert_eq!(rows[0].cwd.as_deref(), Some("/Users/a/one"));
        assert_eq!(rows[0].usage.input, 1);
    }

    #[test]
    fn recent_sessions_with_empty_index_is_empty() {
        let db = open_in_memory().unwrap();
        assert!(recent_sessions(&db, &[], 3).unwrap().is_empty());
    }

    #[test]
    fn recent_sessions_with_limit_zero_is_empty() {
        // A database with real sessions must still yield nothing for
        // limit == 0: an empty database would pass this even with the
        // `out.len() == limit` break condition that never fires at 0.
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-one", "/Users/a/one", false)
            .unwrap();
        seed_session(&db, pid, "s-a", "/Users/a/one", 1_000, 1);
        assert!(recent_sessions(&db, &[], 0).unwrap().is_empty());
    }

    const DAY: i64 = 86_400_000;

    #[test]
    fn session_history_is_newest_first_with_usage_attached() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        seed_session(&db, pid, "old", "/a/p", 1_000, 1_000_000);
        seed_session(&db, pid, "new", "/a/p", 5_000, 2_000_000);

        let rows = session_history(&db, pid).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "old"]);
        assert_eq!(rows[0].usage.input, 2_000_000);
        assert!(
            (rows[0].cost_usd - 30.0).abs() < 1e-9,
            "2 MTok fable input at 15.0"
        );
    }

    #[test]
    fn session_history_for_a_project_with_no_sessions_is_empty_not_an_error() {
        let db = open_in_memory().unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        assert!(session_history(&db, pid).unwrap().is_empty());
    }

    #[test]
    fn session_history_puts_a_null_last_activity_at_last() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        seed_session(&db, pid, "dated", "/a/p", 1_000, 1);
        // A session that has never recorded activity: `last_activity_at` is
        // NULL, not merely old. It must still appear, sorted after every
        // dated session rather than first (NULL DESC would otherwise put it
        // first) and rather than being dropped.
        db.upsert_session(&SessionRecord {
            id: "undated".into(),
            project_id: pid,
            file_path: "/tmp/undated.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: None,
            last_activity_at: None,
            cwd: Some("/a/p".into()),
            git_branch: None,
            cc_version: None,
            message_count: 0,
        })
        .unwrap();

        let rows = session_history(&db, pid).unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["dated", "undated"],
            "the undated session sorts last"
        );
        assert_eq!(rows[1].last_activity_at, None);
    }

    #[test]
    fn daily_usage_includes_quiet_days_as_zeroes() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        let now = 10 * DAY + 3_600_000; // mid-day on day 10
                                        // Activity on day 10 and day 8 only; day 9 must still appear, zeroed.
        seed_session(&db, pid, "s10", "/a/p", 10 * DAY + 1, 1_000_000);
        seed_session(&db, pid, "s8", "/a/p", 8 * DAY + 1, 3_000_000);

        let days = daily_usage(&db, 3, now).unwrap();
        assert_eq!(days.len(), 3, "exactly the window requested");
        assert_eq!(days[0].day_start_ms, 8 * DAY, "oldest first");
        assert_eq!(days[2].day_start_ms, 10 * DAY);
        assert_eq!(days[0].usage.input, 3_000_000);
        assert_eq!(
            days[1].usage.total_tokens(),
            0,
            "a quiet day is present and zeroed"
        );
        assert_eq!(days[1].cost_usd, 0.0);
        assert_eq!(days[2].usage.input, 1_000_000);
    }

    #[test]
    fn daily_usage_for_project_excludes_other_projects_activity() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let a = db.upsert_project("-a-a", "/a/a", false).unwrap();
        let b = db.upsert_project("-a-b", "/a/b", false).unwrap();
        let now = 10 * DAY + 3_600_000; // mid-day on day 10
        seed_session(&db, a, "sa", "/a/a", 10 * DAY + 1, 1_000_000);
        seed_session(&db, b, "sb", "/a/b", 10 * DAY + 1, 9_000_000);

        let days = daily_usage_for_project(&db, a, 3, now).unwrap();
        assert_eq!(days.len(), 3, "exactly the window requested");
        assert_eq!(
            days[2].usage.input, 1_000_000,
            "only project a's tokens, not project b's"
        );

        // Workspace-wide `daily_usage` still sees both, so the two functions
        // are not accidentally the same query.
        let workspace = daily_usage(&db, 3, now).unwrap();
        assert_eq!(workspace[2].usage.input, 10_000_000);
    }

    #[test]
    fn daily_usage_with_an_empty_index_still_returns_the_full_window() {
        let db = open_in_memory().unwrap();
        let days = daily_usage(&db, 14, 100 * DAY).unwrap();
        assert_eq!(days.len(), 14);
        assert!(days.iter().all(|d| d.usage.total_tokens() == 0));
    }

    #[test]
    fn top_projects_ranks_by_tokens_and_respects_the_limit() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let big = db.upsert_project("-a-big", "/a/big", false).unwrap();
        let mid = db.upsert_project("-a-mid", "/a/mid", false).unwrap();
        let small = db.upsert_project("-a-small", "/a/small", false).unwrap();
        seed_session(&db, big, "b", "/a/big", 5_000, 9_000_000);
        seed_session(&db, mid, "m", "/a/mid", 5_000, 5_000_000);
        seed_session(&db, small, "s", "/a/small", 5_000, 1_000_000);

        let rows = top_projects(&db, 0, 2).unwrap();
        assert_eq!(rows.len(), 2, "limit honoured");
        assert_eq!(
            rows[0].0, "big",
            "ranked by tokens, labelled by directory name"
        );
        assert_eq!(rows[1].0, "mid");
        assert!(rows[0].1.input > rows[1].1.input);
    }

    #[test]
    fn top_projects_keeps_distinct_projects_with_the_same_directory_basename_separate() {
        // Two unrelated projects whose directory basename collides — e.g.
        // nested checkouts both leaf-named "app" — must not be summed into
        // one row under the shared label. Neither sets a `display_name`, so
        // both fall back to the basename and would collide if the fold were
        // keyed by label instead of project id.
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let work = db
            .upsert_project("-Users-a-work-app", "/Users/a/work/app", false)
            .unwrap();
        let side = db
            .upsert_project("-Users-a-side-app", "/Users/a/side/app", false)
            .unwrap();
        seed_session(&db, work, "w", "/Users/a/work/app", 5_000, 4_000_000);
        seed_session(&db, side, "s", "/Users/a/side/app", 5_000, 1_000_000);

        let rows = top_projects(&db, 0, 5).unwrap();
        assert_eq!(
            rows.len(),
            2,
            "distinct projects must stay separate rows, not merge into one \"app\""
        );
        assert!(rows.iter().all(|r| r.0 == "app"));
        // fable input @ 15.0/MTok: 4 MTok -> $60, 1 MTok -> $15.
        let mut totals: Vec<u64> = rows.iter().map(|r| r.1.input).collect();
        totals.sort_unstable();
        assert_eq!(totals, vec![1_000_000, 4_000_000]);
        let mut costs: Vec<f64> = rows.iter().map(|r| r.2).collect();
        costs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((costs[0] - 15.0).abs() < 1e-9);
        assert!((costs[1] - 60.0).abs() < 1e-9);
    }

    #[test]
    fn top_projects_excludes_activity_before_the_cutoff() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        seed_session(&db, pid, "old", "/a/p", 1_000, 5_000_000);
        assert!(
            top_projects(&db, 10_000, 5).unwrap().is_empty(),
            "all activity predates the cutoff"
        );
    }
}
