//! Read-side aggregation.

use crate::db::Db;
use crate::model::TurnUsage;
use crate::pricing::{cost_usd, price_for};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};

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
    pub unpriced_tokens: u64,
    pub last_activity_at: Option<i64>,
}

/// Tokens and what they cost, with how many of them have no price. Those tokens
/// add nothing to `cost_usd`, so it is only a lower bound when this isn't zero.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spend {
    pub usage: TurnUsage,
    pub cost_usd: f64,
    pub unpriced_tokens: u64,
}

impl Spend {
    fn add(&mut self, db: &Db, model: &str, usage: &TurnUsage) -> Result<()> {
        self.usage = self.usage.plus(usage);
        match price_for(db, model)? {
            Some(price) => self.cost_usd += cost_usd(&price, usage),
            None => self.unpriced_tokens += usage.total_tokens(),
        }
        Ok(())
    }
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

/// Unknown models contribute tokens but no cost.
fn cost_of(db: &Db, model: &str, usage: &TurnUsage) -> Result<f64> {
    Ok(match price_for(db, model)? {
        Some(p) => cost_usd(&p, usage),
        None => 0.0,
    })
}

/// Reduce per-model usage rows into a total: tokens summed across models, and
/// cost summed after pricing each model's tokens at that model's own rate.
fn spend_of(db: &Db, per_model: &[(String, TurnUsage)]) -> Result<Spend> {
    let mut spend = Spend::default();
    for (model, u) in per_model {
        spend.add(db, model, u)?;
    }
    Ok(spend)
}

/// Whether Perch has a price for this model at all.
pub fn model_has_price(db: &Db, model: &str) -> Result<bool> {
    Ok(price_for(db, model)?.is_some())
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

        let spend = spend_of(db, &per_model)?;

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
            usage: spend.usage,
            cost_usd: spend.cost_usd,
            unpriced_tokens: spend.unpriced_tokens,
            last_activity_at,
        });
    }
    Ok(out)
}

/// Serves both clocks: a trailing 5-hour window and a trailing week differ only
/// in `since_ms`. Pass 0 for all time.
pub fn usage_since(db: &Db, since_ms: i64) -> Result<(TurnUsage, f64)> {
    let spend = spend_since(db, since_ms)?;
    Ok((spend.usage, spend.cost_usd))
}

/// [`usage_since`], with how much of it has no price.
pub fn spend_since(db: &Db, since_ms: i64) -> Result<Spend> {
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns WHERE ts >= ?1 GROUP BY model"),
        &[&since_ms],
    )?;
    spend_of(db, &per_model)
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

/// When the index's records begin: its oldest turn overall, or one project's.
/// Days before it are unknown, not idle.
pub fn first_turn_at(db: &Db, project_id: Option<i64>) -> Result<Option<i64>> {
    let ts: Option<i64> = match project_id {
        Some(id) => db.conn().query_row(
            "SELECT MIN(ts) FROM turns
             WHERE session_id IN (SELECT id FROM sessions WHERE project_id = ?1)",
            params![id],
            |r| r.get(0),
        )?,
        None => db
            .conn()
            .query_row("SELECT MIN(ts) FROM turns", [], |r| r.get(0))?,
    };
    Ok(ts)
}

/// [`usage_by_model`] restricted to turns at or after `since_ms`.
pub fn usage_by_model_since(db: &Db, since_ms: i64) -> Result<Vec<(String, TurnUsage, f64)>> {
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns WHERE ts >= ?1 GROUP BY model"),
        &[&since_ms],
    )?;
    let mut out = Vec::new();
    for (model, u) in per_model {
        let c = cost_of(db, &model, &u)?;
        out.push((model, u, c));
    }
    Ok(out)
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
    /// The session's own title, as recovered from its transcript. `None`
    /// for the handful that never recorded one.
    pub title: Option<String>,
    pub usage: TurnUsage,
    pub cost_usd: f64,
    pub unpriced_tokens: u64,
}

/// The stored titles for the given session ids. Live records carry Claude
/// Code's `<project>-<id>` slug rather than a title, so a shell showing live
/// sessions has to come back here for the name a human would recognise.
pub fn titles_for(db: &Db, ids: &[&str]) -> Result<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let mut stmt = db
        .conn()
        .prepare("SELECT title FROM sessions WHERE id = ?1")?;
    for id in ids {
        let title: Option<String> = stmt
            .query_row([id], |r| r.get::<_, Option<String>>(0))
            .optional()?
            .flatten();
        if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
            out.insert((*id).to_string(), t);
        }
    }
    Ok(out)
}

/// The git branch the index last recorded for one session, or `None` when it
/// has never seen one. Like `titles_for`, an unknown session is simply absent
/// rather than an error: the popover asks about live sessions, and a session
/// that has not been indexed yet is an ordinary, momentary state.
///
/// `git_branch` is LATEST-wins in `scan.rs`, so this is the branch the session
/// was on when it last wrote a turn — which is the branch it is on now,
/// unless the user switched without saying anything to Claude Code.
pub fn session_branch(db: &Db, session_id: &str) -> Result<Option<String>> {
    let branch: Option<String> = db
        .conn()
        .query_row(
            "SELECT git_branch FROM sessions WHERE id = ?1",
            [session_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    Ok(branch.filter(|b| !b.trim().is_empty()))
}

/// Tokens and estimated cost for one session, priced per model and summed.
/// An unknown session is simply zero — it is not an error to ask.
pub fn session_usage(db: &Db, session_id: &str) -> Result<(TurnUsage, f64)> {
    let spend = session_spend(db, session_id)?;
    Ok((spend.usage, spend.cost_usd))
}

/// [`session_usage`], with how much of it has no price.
pub fn session_spend(db: &Db, session_id: &str) -> Result<Spend> {
    let args: [&dyn rusqlite::ToSql; 1] = [&session_id];
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns WHERE session_id = ?1 GROUP BY model"),
        &args,
    )?;
    spend_of(db, &per_model)
}

/// The same turns [`session_usage`] totals, split by the model that spent
/// them and priced at that model's own rate — heaviest first, ties broken by
/// name so the order never depends on SQLite's row order.
///
/// `session_usage` is the sum of exactly this list, and the two share the
/// query and the pricing helper for that reason: a session's per-model rows
/// can never add up to something other than its own total.
pub fn session_usage_by_model(db: &Db, session_id: &str) -> Result<Vec<(String, TurnUsage, f64)>> {
    let args: [&dyn rusqlite::ToSql; 1] = [&session_id];
    let per_model = usage_rows(
        db,
        &format!("SELECT model, {SUMS} FROM turns WHERE session_id = ?1 GROUP BY model"),
        &args,
    )?;
    let mut out = Vec::new();
    for (model, u) in per_model {
        let c = cost_of(db, &model, &u)?;
        out.push((model, u, c));
    }
    out.sort_by(|a, b| {
        b.1.total_tokens()
            .cmp(&a.1.total_tokens())
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(out)
}

/// One subagent's share of a session.
#[derive(Debug, Clone, PartialEq)]
pub struct SubagentUsage {
    pub agent_type: Option<String>,
    pub file_path: String,
    /// Its most recent turn. `None` for a subagent with no turns yet.
    pub last_ts: Option<i64>,
    pub usage: TurnUsage,
    pub cost_usd: f64,
    pub unpriced_tokens: u64,
}

/// Every subagent this session has run, each priced per model and summed.
pub fn session_subagents(db: &Db, session_id: &str) -> Result<Vec<SubagentUsage>> {
    let mut stmt = db.conn().prepare(&format!(
        "SELECT sa.agent_id, sa.agent_type, sa.file_path, t.model, MAX(t.ts), {SUMS}
         FROM subagents sa
         LEFT JOIN turns t ON t.session_id = sa.session_id AND t.agent_id = sa.agent_id
         WHERE sa.session_id = ?1
         GROUP BY sa.agent_id, t.model"
    ))?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                TurnUsage {
                    input: r.get::<_, i64>(5)? as u64,
                    output: r.get::<_, i64>(6)? as u64,
                    cache_read: r.get::<_, i64>(7)? as u64,
                    cache_write_5m: r.get::<_, i64>(8)? as u64,
                    cache_write_1h: r.get::<_, i64>(9)? as u64,
                    thinking: r.get::<_, i64>(10)? as u64,
                },
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut by_agent: std::collections::BTreeMap<String, SubagentUsage> = Default::default();
    for (agent_id, agent_type, file_path, model, last_ts, usage) in rows {
        let mut spend = Spend::default();
        if let Some(m) = &model {
            spend.add(db, m, &usage)?;
        }
        let entry = by_agent.entry(agent_id).or_insert(SubagentUsage {
            agent_type,
            file_path,
            last_ts: None,
            usage: TurnUsage::default(),
            cost_usd: 0.0,
            unpriced_tokens: 0,
        });
        entry.last_ts = entry.last_ts.max(last_ts);
        entry.usage = entry.usage.plus(&spend.usage);
        entry.cost_usd += spend.cost_usd;
        entry.unpriced_tokens += spend.unpriced_tokens;
    }
    Ok(by_agent.into_values().collect())
}

/// One session's own activity over its own lifetime: its turns bucketed into
/// `buckets` equal slices of the span between its first turn and its last,
/// oldest first, quiet slices present and zeroed.
///
/// Deliberately *not* the calendar days [`daily_usage_for_project`] uses. A
/// session lives for minutes or hours far more often than for days, and a
/// day-per-bar chart of one would be a single bar — which says nothing about
/// the shape of the session, which is the whole reason to draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionActivity {
    pub first_ts: i64,
    pub last_ts: i64,
    /// Exactly `buckets` entries.
    pub buckets: Vec<TurnUsage>,
}

/// `None` when the index holds no turns for this session at all — which a
/// caller must say in words rather than draw as a flat line at zero.
pub fn session_activity(
    db: &Db,
    session_id: &str,
    buckets: usize,
) -> Result<Option<SessionActivity>> {
    let buckets = buckets.max(1);
    let bounds: Option<(Option<i64>, Option<i64>)> = db
        .conn()
        .query_row(
            "SELECT MIN(ts), MAX(ts) FROM turns WHERE session_id = ?1",
            params![session_id],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?;
    let Some((Some(first_ts), Some(last_ts))) = bounds else {
        return Ok(None);
    };

    let mut stmt = db.conn().prepare(&format!(
        "SELECT ts, {SUMS} FROM turns WHERE session_id = ?1 GROUP BY ts ORDER BY ts"
    ))?;
    let rows = stmt
        .query_map(params![session_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                TurnUsage {
                    input: r.get::<_, i64>(1)? as u64,
                    output: r.get::<_, i64>(2)? as u64,
                    cache_read: r.get::<_, i64>(3)? as u64,
                    cache_write_5m: r.get::<_, i64>(4)? as u64,
                    cache_write_1h: r.get::<_, i64>(5)? as u64,
                    thinking: r.get::<_, i64>(6)? as u64,
                },
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let span = last_ts - first_ts;
    let mut out = vec![TurnUsage::default(); buckets];
    for (ts, u) in rows {
        // A session whose turns all share one timestamp has no span to divide;
        // everything it did belongs to the one slice there is.
        let i = if span <= 0 {
            0
        } else {
            // The last turn lands in the last bucket rather than one past it.
            (((ts - first_ts) as i128 * buckets as i128) / span as i128).min(buckets as i128 - 1)
                as usize
        };
        out[i] = out[i].plus(&u);
    }
    Ok(Some(SessionActivity {
        first_ts,
        last_ts,
        buckets: out,
    }))
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
        "SELECT id, cwd, last_activity_at, title FROM sessions
         WHERE last_activity_at IS NOT NULL
         ORDER BY last_activity_at DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, cwd, last, title) = row?;
        if exclude_ids.iter().any(|x| x == &id) {
            continue;
        }
        let spend = session_spend(db, &id)?;
        out.push(RecentSession {
            id,
            cwd,
            last_activity_at: last,
            title,
            usage: spend.usage,
            cost_usd: spend.cost_usd,
            unpriced_tokens: spend.unpriced_tokens,
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
    pub title: Option<String>,
    pub usage: TurnUsage,
    pub cost_usd: f64,
    pub unpriced_tokens: u64,
}

/// Every session ever recorded for a project, newest first. Sessions with no
/// recorded activity sort last rather than being dropped — they exist, and the
/// window's job is to show what exists.
pub fn session_history(db: &Db, project_id: i64) -> Result<Vec<HistorySession>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, started_at, last_activity_at, git_branch, message_count, title
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
            r.get::<_, Option<String>>(5)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, started_at, last_activity_at, git_branch, message_count, title) = row?;
        let spend = session_spend(db, &id)?;
        out.push(HistorySession {
            id,
            started_at,
            last_activity_at,
            git_branch,
            message_count,
            title,
            usage: spend.usage,
            cost_usd: spend.cost_usd,
            unpriced_tokens: spend.unpriced_tokens,
        });
    }
    Ok(out)
}

/// One calendar day in the user's time zone. Two measures answer different
/// questions, and neither is total tokens, which is almost all cache reads and
/// so mostly shows how long the conversations had grown:
/// - `responses` is how active the day was. It needs no price and cache size
///   doesn't move it.
/// - `cost_usd` is what the day would cost, with `usage.output` as the stand-in
///   when nothing has a price.
#[derive(Debug, Clone)]
pub struct DayUsage {
    pub date: chrono::NaiveDate,
    /// API responses, subagents' included.
    pub responses: u64,
    /// Sessions with at least one response that day.
    pub sessions: u64,
    pub usage: TurnUsage,
    pub cost_usd: f64,
    pub unpriced_tokens: u64,
    /// The same day split by model, for charts stacked by model.
    pub by_model: Vec<ModelDay>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelDay {
    pub model: String,
    pub usage: TurnUsage,
    /// `None` when the model has no price.
    pub cost_usd: Option<f64>,
}

/// A `WITH` clause naming the window's days, so one query can group by them.
/// A day's length varies, so SQL can't work out which day a turn is in from its
/// timestamp alone. Every value is an integer computed here, never user input.
fn days_table(window: &[crate::days::Day]) -> String {
    let rows = window
        .iter()
        .enumerate()
        .map(|(i, d)| format!("({i},{},{})", d.start_ms, d.end_ms))
        .collect::<Vec<_>>()
        .join(",");
    format!("WITH days(i, lo, hi) AS (VALUES {rows})")
}

/// Shared by `daily_usage` and `daily_usage_for_project`: everything except
/// which turns are in scope (all of them, or one project's) is identical.
pub(crate) fn daily_usage_in<Tz: chrono::TimeZone>(
    db: &Db,
    tz: &Tz,
    days: usize,
    now_ms: i64,
    project_filter: Option<i64>,
) -> Result<Vec<DayUsage>> {
    let window = crate::days::window(tz, days, now_ms);
    if window.is_empty() {
        return Ok(Vec::new());
    }
    let scope = match project_filter {
        Some(_) => "AND session_id IN (SELECT id FROM sessions WHERE project_id = ?3)",
        None => "",
    };
    // One range scan, grouped here rather than in SQL: a year of turns is too
    // many for SQLite to sort into days quickly, and a day's length varies, so
    // SQL can't tell which day a turn is in from its timestamp alone.
    let mut stmt = db.conn().prepare(&format!(
        "SELECT ts, model, session_id, input, output, cache_read, cache_write_5m,
                cache_write_1h, thinking
         FROM turns WHERE ts >= ?1 AND ts < ?2 {scope}"
    ))?;
    let lo = window[0].start_ms;
    let hi = window[window.len() - 1].end_ms;
    let mut rows = match project_filter {
        Some(project_id) => stmt.query(params![lo, hi, project_id])?,
        None => stmt.query(params![lo, hi])?,
    };

    struct Slot {
        responses: u64,
        by_model: Vec<(String, TurnUsage)>,
        sessions: std::collections::HashSet<String>,
    }
    let mut slots: Vec<Slot> = window
        .iter()
        .map(|_| Slot {
            responses: 0,
            by_model: Vec::new(),
            sessions: std::collections::HashSet::new(),
        })
        .collect();
    while let Some(r) = rows.next()? {
        let ts: i64 = r.get(0)?;
        let slot = &mut slots[window.partition_point(|d| d.end_ms <= ts)];
        let model = r.get_ref(1)?.as_str()?;
        let session = r.get_ref(2)?.as_str()?;
        let usage = TurnUsage {
            input: r.get::<_, i64>(3)? as u64,
            output: r.get::<_, i64>(4)? as u64,
            cache_read: r.get::<_, i64>(5)? as u64,
            cache_write_5m: r.get::<_, i64>(6)? as u64,
            cache_write_1h: r.get::<_, i64>(7)? as u64,
            thinking: r.get::<_, i64>(8)? as u64,
        };
        slot.responses += 1;
        match slot.by_model.iter_mut().find(|(m, _)| m == model) {
            Some((_, total)) => *total = total.plus(&usage),
            None => slot.by_model.push((model.to_owned(), usage)),
        }
        if !slot.sessions.contains(session) {
            slot.sessions.insert(session.to_owned());
        }
    }
    drop(rows);

    window
        .iter()
        .zip(slots)
        .map(|(day, slot)| {
            let mut spend = Spend::default();
            let mut by_model = Vec::with_capacity(slot.by_model.len());
            for (model, usage) in slot.by_model {
                spend.add(db, &model, &usage)?;
                let cost_usd = price_for(db, &model)?.map(|p| cost_usd(&p, &usage));
                by_model.push(ModelDay {
                    model,
                    usage,
                    cost_usd,
                });
            }
            Ok(DayUsage {
                date: day.date,
                responses: slot.responses,
                sessions: slot.sessions.len() as u64,
                usage: spend.usage,
                cost_usd: spend.cost_usd,
                unpriced_tokens: spend.unpriced_tokens,
                by_model,
            })
        })
        .collect()
}

/// `days` consecutive local days ending with the one containing `now_ms`, oldest
/// first. Quiet days are present and zeroed: a bar chart with gaps silently lies
/// about the shape of the week.
pub fn daily_usage(db: &Db, days: usize, now_ms: i64) -> Result<Vec<DayUsage>> {
    daily_usage_in(db, &chrono::Local, days, now_ms, None)
}

/// Same shape as `daily_usage`, scoped to one project: the main window's
/// per-project sparkline must not silently show every project's activity on
/// a single project's page.
pub fn daily_usage_for_project(
    db: &Db,
    project_id: i64,
    days: usize,
    now_ms: i64,
) -> Result<Vec<DayUsage>> {
    daily_usage_in(db, &chrono::Local, days, now_ms, Some(project_id))
}

/// Every project's responses per local day over the same window as
/// `daily_usage_for_project`: one grouped query rather than one per project, for
/// the overview's cards. Each vector is `days` long, oldest first, quiet days
/// zeroed; a project with no turns in the window is simply absent from the map.
pub fn daily_responses_by_project(
    db: &Db,
    days: usize,
    now_ms: i64,
) -> Result<std::collections::HashMap<i64, Vec<u64>>> {
    daily_responses_by_project_in(db, &chrono::Local, days, now_ms)
}

fn daily_responses_by_project_in<Tz: chrono::TimeZone>(
    db: &Db,
    tz: &Tz,
    days: usize,
    now_ms: i64,
) -> Result<std::collections::HashMap<i64, Vec<u64>>> {
    let mut out: std::collections::HashMap<i64, Vec<u64>> = std::collections::HashMap::new();
    let window = crate::days::window(tz, days, now_ms);
    if window.is_empty() {
        return Ok(out);
    }
    let mut stmt = db.conn().prepare(&format!(
        "{} SELECT d.i, s.project_id, COUNT(*)
         FROM days d CROSS JOIN turns t
         JOIN sessions s ON s.id = t.session_id
         WHERE t.ts >= d.lo AND t.ts < d.hi
         GROUP BY d.i, s.project_id",
        days_table(&window)
    ))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)? as usize,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)? as u64,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (slot, project_id, responses) in rows {
        out.entry(project_id)
            .or_insert_with(|| vec![0; window.len()])[slot] = responses;
    }
    Ok(out)
}

/// Each project's branch, from its most recently active session that recorded
/// a non-blank one. A project that never ran inside a git checkout is absent.
pub fn latest_branches(db: &Db) -> Result<std::collections::HashMap<i64, String>> {
    let mut stmt = db.conn().prepare(
        "SELECT project_id, git_branch FROM sessions
         WHERE git_branch IS NOT NULL AND TRIM(git_branch) <> ''
         ORDER BY last_activity_at IS NULL, last_activity_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut out = std::collections::HashMap::new();
    for (project_id, branch) in rows {
        // Rows arrive newest first, so the first branch seen per project wins.
        out.entry(project_id).or_insert(branch);
    }
    Ok(out)
}

/// Projects ranked by tokens since `since_ms`. The label is the directory name,
/// which is what the user recognises — not the slug and not the whole path.
pub fn top_projects(db: &Db, since_ms: i64, limit: usize) -> Result<Vec<(String, Spend)>> {
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
    let mut totals: std::collections::HashMap<i64, (String, Spend)> =
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
        let slot = totals
            .entry(project_id)
            .or_insert_with(|| (label.clone(), Spend::default()));
        slot.0 = label;
        slot.1.add(db, &model, &u)?;
    }

    let mut out: Vec<(String, Spend)> = totals.into_values().collect();
    out.sort_by(|a, b| {
        b.1.usage
            .total_tokens()
            .cmp(&a.1.usage.total_tokens())
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
            provider: crate::provider::CLAUDE_CODE.into(),
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
            title: None,
            message_count: 2,
        })
        .unwrap();
        db.insert_turns(
            "s1",
            &[
                Turn {
                    message_id: None,
                    ts: 1_000,
                    model: "claude-fable-5".into(),
                    usage: TurnUsage {
                        input: 1_000_000,
                        ..Default::default()
                    },
                },
                Turn {
                    message_id: None,
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
        assert!((r.cost_usd - 15.0).abs() < 1e-9);
        assert_eq!(r.unpriced_tokens, 0);
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
                message_id: None,
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
            (rows[0].cost_usd - 15.0).abs() < 1e-9,
            "unpriced model adds no cost"
        );
        assert_eq!(
            rows[0].unpriced_tokens, 2_000_000,
            "and the gap is recorded, so the cost can say it is a lower bound"
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
            provider: crate::provider::CLAUDE_CODE.into(),
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
            title: None,
            message_count: 1,
        })
        .unwrap();
        db.insert_turns(
            id,
            &[Turn {
                message_id: None,
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
    fn session_branch_is_the_one_the_index_recorded_and_nothing_when_it_has_none() {
        let db = open_in_memory().unwrap();
        let pid = db
            .upsert_project("-Users-a-one", "/Users/a/one", false)
            .unwrap();

        // No branch recorded at all, a recorded branch, and a blank one —
        // which the transcript writes when the session is not in a git
        // checkout. All three must read as "Perch does not know", so the
        // detail row shows an em dash rather than an empty label.
        for (id, branch) in [
            ("s-none", None),
            ("s-main", Some("main".to_string())),
            ("s-blank", Some("   ".to_string())),
        ] {
            db.upsert_session(&SessionRecord {
                provider: crate::provider::CLAUDE_CODE.into(),
                id: id.into(),
                project_id: pid,
                file_path: format!("/tmp/{id}.jsonl"),
                file_size: 0,
                indexed_offset: 0,
                started_at: Some(1_000),
                last_activity_at: Some(2_000),
                cwd: None,
                git_branch: branch,
                cc_version: None,
                title: None,
                message_count: 1,
            })
            .unwrap();
        }

        assert_eq!(
            session_branch(&db, "s-main").unwrap().as_deref(),
            Some("main")
        );
        assert_eq!(session_branch(&db, "s-none").unwrap(), None);
        assert_eq!(session_branch(&db, "s-blank").unwrap(), None);
        assert_eq!(
            session_branch(&db, "never-indexed").unwrap(),
            None,
            "asking about a session the index has never seen is not an error"
        );
    }

    #[test]
    fn session_usage_prices_per_model() {
        // A single-model fixture can't prove per-model pricing is applied (a bug
        // that resolved the wrong model's price could still pass). Use two
        // models with different input rates: fable at $10/MTok, sonnet at $2.
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-one", "/Users/a/one", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            provider: crate::provider::CLAUDE_CODE.into(),
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
            title: None,
            message_count: 2,
        })
        .unwrap();
        db.insert_turns(
            "s-a",
            &[
                Turn {
                    message_id: None,
                    ts: 9_000,
                    model: "claude-fable-5".into(),
                    usage: TurnUsage {
                        input: 2_000_000,
                        ..Default::default()
                    },
                },
                Turn {
                    message_id: None,
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
        // Reachable only if fable's 2 MTok is priced at $10/MTok ($20.00) and
        // sonnet's 1 MTok is priced at its own $2/MTok ($2.00), summed.
        // Pooling both under either rate would give a different total.
        assert!(
            (cost - 22.0).abs() < 1e-9,
            "fable 2 MTok @ 10.0 + sonnet 1 MTok @ 2.0 = 22.00"
        );
    }

    /// A session with turns at `ts`, each one model's, so the two new
    /// per-session queries have something with real shape to read.
    fn seed_session_turns(db: &Db, id: &str, turns: &[(i64, &str, TurnUsage)]) {
        let pid = db
            .upsert_project("-Users-a-one", "/Users/a/one", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            provider: crate::provider::CLAUDE_CODE.into(),
            id: id.into(),
            project_id: pid,
            file_path: format!("/tmp/{id}.jsonl"),
            file_size: 0,
            indexed_offset: 0,
            started_at: turns.first().map(|t| t.0),
            last_activity_at: turns.last().map(|t| t.0),
            cwd: Some("/Users/a/one".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: turns.len() as u64,
        })
        .unwrap();
        let turns: Vec<Turn> = turns
            .iter()
            .map(|(ts, model, usage)| Turn {
                message_id: None,
                ts: *ts,
                model: (*model).into(),
                usage: *usage,
            })
            .collect();
        db.insert_turns(id, &turns).unwrap();
    }

    fn input(n: u64) -> TurnUsage {
        TurnUsage {
            input: n,
            ..Default::default()
        }
    }

    #[test]
    fn session_usage_by_model_is_heaviest_first_and_priced_per_model() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        // Inserted lightest-first so a query that simply echoed SQLite's row
        // order would come back the wrong way round.
        seed_session_turns(
            &db,
            "s-m",
            &[
                (1_000, "claude-sonnet-5", input(1_000_000)),
                (2_000, "claude-fable-5", input(2_000_000)),
            ],
        );
        let rows = session_usage_by_model(&db, "s-m").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "claude-fable-5", "heaviest model first");
        assert_eq!(rows[1].0, "claude-sonnet-5");
        // Fable at $15.0/MTok, sonnet at its own $3.0/MTok — pooling both
        // under either rate would give different numbers here.
        assert!((rows[0].2 - 20.0).abs() < 1e-9, "2 MTok @ 10.0");
        assert!((rows[1].2 - 2.0).abs() < 1e-9, "1 MTok @ 2.0");
        // The split must add up to exactly what the session's own total says.
        let (total, cost) = session_usage(&db, "s-m").unwrap();
        assert_eq!(
            rows.iter().map(|r| r.1.total_tokens()).sum::<u64>(),
            total.total_tokens()
        );
        assert!((rows.iter().map(|r| r.2).sum::<f64>() - cost).abs() < 1e-9);
    }

    #[test]
    fn session_usage_by_model_for_an_unknown_session_is_empty_not_error() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        assert!(session_usage_by_model(&db, "nope").unwrap().is_empty());
    }

    #[test]
    fn session_activity_spreads_turns_across_its_own_lifetime() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        // First, middle and last of a 100-second life. With four buckets the
        // 25-second mark belongs to bucket 1 and the 50-second mark to bucket
        // 2; the last turn must land in the last bucket, not one past it.
        seed_session_turns(
            &db,
            "s-t",
            &[
                (100_000, "claude-fable-5", input(10)),
                (125_000, "claude-fable-5", input(20)),
                (150_000, "claude-fable-5", input(40)),
                (200_000, "claude-fable-5", input(80)),
            ],
        );
        let a = session_activity(&db, "s-t", 4).unwrap().unwrap();
        assert_eq!(a.first_ts, 100_000);
        assert_eq!(a.last_ts, 200_000);
        assert_eq!(a.buckets.len(), 4);
        let tokens: Vec<u64> = a.buckets.iter().map(|u| u.total_tokens()).collect();
        assert_eq!(
            tokens,
            vec![10, 20, 40, 80],
            "each turn belongs to the slice of the session's life it happened in"
        );
    }

    #[test]
    fn session_activity_keeps_quiet_slices_and_sums_the_whole_session() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        seed_session_turns(
            &db,
            "s-q",
            &[
                (0, "claude-fable-5", input(100)),
                (1_000_000, "claude-fable-5", input(300)),
            ],
        );
        let a = session_activity(&db, "s-q", 5).unwrap().unwrap();
        let tokens: Vec<u64> = a.buckets.iter().map(|u| u.total_tokens()).collect();
        assert_eq!(
            tokens,
            vec![100, 0, 0, 0, 300],
            "a quiet stretch is a zero in place, not a missing bucket"
        );
        // Nothing may be dropped on the way into the buckets.
        let (total, _) = session_usage(&db, "s-q").unwrap();
        assert_eq!(
            a.buckets.iter().map(|u| u.total_tokens()).sum::<u64>(),
            total.total_tokens()
        );
    }

    #[test]
    fn session_activity_of_a_session_with_one_instant_does_not_divide_by_zero() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        seed_session_turns(&db, "s-1", &[(7_000, "claude-fable-5", input(55))]);
        let a = session_activity(&db, "s-1", 6).unwrap().unwrap();
        assert_eq!(a.first_ts, a.last_ts);
        assert_eq!(a.buckets.len(), 6);
        assert_eq!(a.buckets[0].total_tokens(), 55);
        assert_eq!(
            a.buckets[1..].iter().map(|u| u.total_tokens()).sum::<u64>(),
            0
        );
    }

    #[test]
    fn session_activity_of_a_session_with_no_turns_is_nothing_not_zeroes() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        assert!(
            session_activity(&db, "never-indexed", 8).unwrap().is_none(),
            "a session with nothing recorded must be distinguishable from one \
             that was recorded as quiet"
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
            (rows[0].cost_usd - 20.0).abs() < 1e-9,
            "2 MTok fable input at 15.0"
        );
    }

    #[test]
    fn session_history_carries_the_stored_title() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        seed_session(&db, pid, "titled", "/a/p", 1_000, 1);
        let mut s = crate::model::SessionRecord {
            provider: crate::provider::CLAUDE_CODE.into(),
            id: "titled".into(),
            project_id: pid,
            file_path: "/tmp/titled.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(1_000),
            last_activity_at: Some(1_000),
            cwd: Some("/a/p".into()),
            git_branch: None,
            cc_version: None,
            title: Some("Claude projects dashboard".into()),
            message_count: 1,
        };
        db.upsert_session(&s).unwrap();

        let rows = session_history(&db, pid).unwrap();
        assert_eq!(rows[0].title.as_deref(), Some("Claude projects dashboard"));

        // A session with no title yields `None`, not an empty string.
        s.id = "untitled".into();
        s.title = None;
        db.upsert_session(&s).unwrap();
        let rows = session_history(&db, pid).unwrap();
        let untitled = rows.iter().find(|r| r.id == "untitled").unwrap();
        assert_eq!(untitled.title, None);
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
            provider: crate::provider::CLAUDE_CODE.into(),
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
            title: None,
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

        let days = daily_usage_in(&db, &chrono::Utc, 3, now, None).unwrap();
        assert_eq!(days.len(), 3, "exactly the window requested");
        assert_eq!(
            days[0]
                .date
                .and_time(chrono::NaiveTime::MIN)
                .and_utc()
                .timestamp_millis(),
            8 * DAY,
            "oldest first"
        );
        assert_eq!(days[1].date.succ_opt(), Some(days[2].date));
        assert_eq!(days[0].usage.input, 3_000_000);
        assert_eq!(
            days[1].usage.total_tokens(),
            0,
            "a quiet day is present and zeroed"
        );
        assert_eq!(days[1].cost_usd, 0.0);
        assert_eq!(days[1].responses, 0);
        assert_eq!(days[2].usage.input, 1_000_000);
        assert_eq!(days[2].responses, 1);
        assert_eq!(days[2].sessions, 1);
    }

    #[test]
    fn a_turn_counts_on_the_local_day_it_happened() {
        use crate::days::test_zones::Zone;
        use chrono::{NaiveDate, TimeZone};

        let ms = |tz: &Zone, m, d, h, min| {
            tz.with_ymd_and_hms(2026, m, d, h, min, 0)
                .unwrap()
                .timestamp_millis()
        };
        let date = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();

        // (zone, the turn's month, day, hour and minute). Each is on a different
        // UTC date from its local one, or on a day with a clock change.
        for (tz, m, d, h, min) in [
            (Zone::Havana, 1, 15, 23, 30),  // already the 16th in UTC
            (Zone::Berlin, 1, 15, 0, 30),   // still the 14th in UTC
            (Zone::Berlin, 3, 29, 23, 30),  // 23-hour day
            (Zone::Berlin, 3, 29, 0, 30),   // before the clocks go forward
            (Zone::Berlin, 10, 25, 23, 30), // 25-hour day
            (Zone::Havana, 3, 8, 23, 30),   // a day with no midnight
        ] {
            let db = open_in_memory().unwrap();
            seed_default_prices(&db).unwrap();
            let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
            let ts = ms(&tz, m, d, h, min);
            seed_session(&db, pid, "s", "/a/p", ts, 1_000);

            // Read from the next day, so the turn's own day is mid-window.
            let now = ms(&tz, m, d, 12, 0) + DAY;
            let days = daily_usage_in(&db, &tz, 3, now, None).unwrap();
            let counted: Vec<_> = days
                .iter()
                .filter(|day| day.responses > 0)
                .map(|day| day.date)
                .collect();
            assert_eq!(counted, vec![date(m, d)], "{tz:?} {m}/{d} {h}:{min}");

            let by_project = daily_responses_by_project_in(&db, &tz, 3, now).unwrap();
            let per_day: Vec<u64> = days.iter().map(|day| day.responses).collect();
            assert_eq!(by_project[&pid], per_day, "both queries pick the same day");
        }
    }

    #[test]
    fn responses_ignore_how_big_the_conversation_had_grown() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        let now = 10 * DAY + 3_600_000;
        seed_session(&db, pid, "huge", "/a/p", 9 * DAY + 1, 90_000_000);
        seed_session(&db, pid, "small", "/a/p", 10 * DAY + 1, 10);

        let days = daily_usage_in(&db, &chrono::Utc, 2, now, None).unwrap();
        assert_eq!(days[0].responses, days[1].responses);
        assert!(days[0].usage.total_tokens() > days[1].usage.total_tokens());
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

        let days = daily_usage_in(&db, &chrono::Utc, 3, now, Some(a)).unwrap();
        assert_eq!(days.len(), 3, "exactly the window requested");
        assert_eq!(
            days[2].usage.input, 1_000_000,
            "only project a's tokens, not project b's"
        );

        // Workspace-wide `daily_usage` still sees both, so the two functions
        // are not accidentally the same query.
        let workspace = daily_usage_in(&db, &chrono::Utc, 3, now, None).unwrap();
        assert_eq!(workspace[2].usage.input, 10_000_000);
    }

    #[test]
    fn daily_responses_by_project_agrees_with_each_projects_own_sparkline() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let a = db.upsert_project("-a-a", "/a/a", false).unwrap();
        let b = db.upsert_project("-a-b", "/a/b", false).unwrap();
        let quiet = db.upsert_project("-a-q", "/a/q", false).unwrap();
        let now = 10 * DAY + 3_600_000;
        seed_session(&db, a, "sa1", "/a/a", 10 * DAY + 1, 1_000);
        seed_session(&db, a, "sa2", "/a/a", 8 * DAY + 5, 700);
        seed_session(&db, b, "sb", "/a/b", 9 * DAY + 1, 9_000);
        // Outside the window: must not be folded into day 0.
        seed_session(&db, quiet, "sq", "/a/q", 2 * DAY, 5_000);

        let all = daily_responses_by_project_in(&db, &chrono::Utc, 3, now).unwrap();
        for pid in [a, b] {
            let own: Vec<u64> = daily_usage_in(&db, &chrono::Utc, 3, now, Some(pid))
                .unwrap()
                .iter()
                .map(|d| d.responses)
                .collect();
            assert_eq!(all[&pid], own, "project {pid}");
        }
        assert_eq!(all[&a], vec![1, 0, 1]);
        assert!(
            !all.contains_key(&quiet),
            "no turns in the window, no entry"
        );
    }

    #[test]
    fn latest_branches_take_the_newest_session_that_recorded_one() {
        let db = open_in_memory().unwrap();
        let p = db.upsert_project("-a-p", "/a/p", false).unwrap();
        let bare = db.upsert_project("-a-bare", "/a/bare", false).unwrap();
        for (id, pid, last, branch) in [
            ("old", p, 1_000, Some("main")),
            ("mid", p, 2_000, Some("feature")),
            ("new-blank", p, 3_000, Some("  ")),
            ("new-none", p, 4_000, None),
            ("bare", bare, 5_000, None),
        ] {
            db.upsert_session(&SessionRecord {
                provider: crate::provider::CLAUDE_CODE.into(),
                id: id.into(),
                project_id: pid,
                file_path: format!("/tmp/{id}.jsonl"),
                file_size: 0,
                indexed_offset: 0,
                started_at: Some(last - 10),
                last_activity_at: Some(last),
                cwd: None,
                git_branch: branch.map(str::to_string),
                cc_version: None,
                title: None,
                message_count: 1,
            })
            .unwrap();
        }
        let branches = latest_branches(&db).unwrap();
        assert_eq!(branches.get(&p).map(String::as_str), Some("feature"));
        assert!(!branches.contains_key(&bare));
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
        assert!(rows[0].1.usage.input > rows[1].1.usage.input);
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
        // fable input @ 10.0/MTok: 4 MTok -> $40, 1 MTok -> $10.
        let mut totals: Vec<u64> = rows.iter().map(|r| r.1.usage.input).collect();
        totals.sort_unstable();
        assert_eq!(totals, vec![1_000_000, 4_000_000]);
        let mut costs: Vec<f64> = rows.iter().map(|r| r.1.cost_usd).collect();
        costs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((costs[0] - 10.0).abs() < 1e-9);
        assert!((costs[1] - 40.0).abs() < 1e-9);
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
