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
}
