//! The usage view's view-model. Chart values cross as numbers; every label is
//! finished here.

use crate::db::Db;
use crate::query::{self, DAY_MS};
use crate::ui::format::{human_cost, human_tokens};
use rusqlite::params;

const CHART_DAYS: usize = 14;
const TOP_N: usize = 8;
const WINDOW_MS: i64 = 5 * 3_600_000;
const MIN_PROJECTABLE_MS: i64 = 30 * 60_000;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HeroStat {
    pub label: String,
    pub value: String,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DailyBar {
    pub day_index: i32,
    pub label: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub thinking: u64,
    pub total_label: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RankedProject {
    pub name: String,
    pub tokens: u64,
    pub tokens_label: String,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ModelUsage {
    pub model: String,
    pub tokens: String,
    pub cost: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageModel {
    pub hero: Vec<HeroStat>,
    pub daily: Vec<DailyBar>,
    pub top_projects: Vec<RankedProject>,
    pub by_model: Vec<ModelUsage>,
    pub burn_rate: Option<String>,
    pub has_data: bool,
}

/// Projected tokens per hour for the current 5-hour window, or `None` when the
/// window is too young or too quiet to project from honestly. Deliberately not
/// a percentage: the ceiling needs the tier-1 usage endpoint, and the original
/// spec §8 forbids inventing one.
///
/// `elapsed_ms` is measured from the oldest turn actually seen inside the
/// window, not from an epoch-aligned boundary. Anthropic's real 5-hour
/// rate-limit windows start at first use, not at a multiple of `WINDOW_MS`
/// since the Unix epoch — Perch has no way to know when the server-side
/// window actually opened, so pretending otherwise (e.g. via
/// `now_ms.rem_euclid(WINDOW_MS)`) would silently imply knowledge Perch does
/// not have. Anchoring elapsed time to the earliest observed turn is
/// conservative (it can only *understate* elapsed time, since the real window
/// may have opened even earlier) and never overstates the confidence of the
/// projection.
fn burn_rate(window_tokens: u64, elapsed_ms: i64) -> Option<String> {
    if window_tokens == 0 || elapsed_ms < MIN_PROJECTABLE_MS {
        return None;
    }
    let hours = elapsed_ms as f64 / 3_600_000.0;
    let per_hour = (window_tokens as f64 / hours).round() as u64;
    Some(format!("≈{}/h at this pace", human_tokens(per_hour)))
}

/// How long ago the oldest turn since `since_ms` happened, relative to
/// `now_ms` — i.e. how much of the trailing window Perch actually has turns
/// for. Zero when the window is empty.
fn observed_elapsed_ms(db: &Db, since_ms: i64, now_ms: i64) -> anyhow::Result<i64> {
    let earliest: Option<i64> = db.conn().query_row(
        "SELECT MIN(ts) FROM turns WHERE ts >= ?1",
        params![since_ms],
        |r| r.get(0),
    )?;
    Ok(match earliest {
        Some(ts) => now_ms - ts,
        None => 0,
    })
}

pub fn build_usage(db: &Db, now_ms: i64) -> anyhow::Result<UsageModel> {
    let window_since = now_ms - WINDOW_MS;
    let (window, window_cost) = query::usage_since(db, window_since)?;
    let (day, day_cost) = query::usage_since(db, now_ms - DAY_MS)?;
    let (week, _) = query::usage_since(db, now_ms - 7 * DAY_MS)?;

    let days = query::daily_usage(db, CHART_DAYS, now_ms)?;
    let daily: Vec<DailyBar> = days
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let days_ago = CHART_DAYS - 1 - i;
            DailyBar {
                day_index: i as i32,
                label: if days_ago == 0 {
                    "Today".to_string()
                } else {
                    format!("{days_ago}d")
                },
                input: d.usage.input,
                output: d.usage.output,
                cache_read: d.usage.cache_read,
                cache_write: d.usage.cache_write_total(),
                thinking: d.usage.thinking,
                total_label: human_tokens(d.usage.total_tokens()),
            }
        })
        .collect();

    let top_projects = query::top_projects(db, now_ms - 30 * DAY_MS, TOP_N)?
        .into_iter()
        .map(|(name, u, cost)| RankedProject {
            name,
            tokens: u.total_tokens(),
            tokens_label: human_tokens(u.total_tokens()),
            cost: human_cost(cost),
        })
        .collect();

    let mut by_model_rows = query::usage_by_model(db)?;
    by_model_rows.sort_by_key(|a| std::cmp::Reverse(a.1.total_tokens()));
    let by_model = by_model_rows
        .into_iter()
        .map(|(model, u, cost)| ModelUsage {
            model,
            tokens: human_tokens(u.total_tokens()),
            cost: human_cost(cost),
        })
        .collect();

    let elapsed_in_window = observed_elapsed_ms(db, window_since, now_ms)?;

    let hero = vec![
        HeroStat {
            label: "5-hour window".into(),
            value: human_tokens(window.total_tokens()),
            caption: Some(human_cost(window_cost)),
        },
        HeroStat {
            label: "Week".into(),
            value: human_tokens(week.total_tokens()),
            caption: None,
        },
        HeroStat {
            label: "24 hours".into(),
            value: human_tokens(day.total_tokens()),
            caption: Some(human_cost(day_cost)),
        },
    ];

    Ok(UsageModel {
        has_data: db.turn_count()? > 0,
        burn_rate: burn_rate(window.total_tokens(), elapsed_in_window),
        hero,
        daily,
        top_projects,
        by_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn, TurnUsage};
    use crate::pricing::seed_default_prices;

    fn seed(db: &Db, sid: &str, ts: i64, model: &str, u: TurnUsage) {
        let pid = db.upsert_project("-a-p", "/a/proj", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: sid.into(),
            project_id: pid,
            file_path: format!("/tmp/{sid}.jsonl"),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(ts),
            last_activity_at: Some(ts),
            cwd: Some("/a/proj".into()),
            git_branch: None,
            cc_version: None,
            message_count: 1,
        })
        .unwrap();
        db.insert_turns(
            sid,
            &[Turn {
                ts,
                model: model.into(),
                usage: u,
            }],
        )
        .unwrap();
    }

    #[test]
    fn an_empty_index_reports_no_data_rather_than_zeroes() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let m = build_usage(&db, 100 * DAY_MS).unwrap();
        assert!(!m.has_data);
        assert!(m.burn_rate.is_none(), "no projection without data");
        assert!(m.top_projects.is_empty());
        assert_eq!(m.daily.len(), 14, "the window is still drawn, just empty");
    }

    #[test]
    fn daily_bars_carry_each_token_class_separately() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY_MS + 3_600_000;
        seed(
            &db,
            "s1",
            100 * DAY_MS + 1,
            "claude-fable-5",
            TurnUsage {
                input: 10,
                output: 20,
                cache_read: 30,
                cache_write_5m: 40,
                cache_write_1h: 5,
                thinking: 6,
            },
        );

        let m = build_usage(&db, now).unwrap();
        let today = m.daily.last().unwrap();
        assert_eq!(today.input, 10);
        assert_eq!(today.output, 20);
        assert_eq!(today.cache_read, 30);
        assert_eq!(
            today.cache_write, 45,
            "5m and 1h cache writes are one visual class"
        );
        assert_eq!(today.thinking, 6);
        // `total_tokens()` excludes `thinking` (it's a subset of `output`, not
        // an additional billable class), so the total is 10+20+30+40+5 = 105,
        // not 111 — double-counting thinking tokens would overstate usage.
        assert_eq!(today.total_label, human_tokens(105));
        assert_eq!(
            today.label, "Today",
            "the last bar reads as today, not \"0d\""
        );
        assert!(m.has_data);
    }

    #[test]
    fn top_projects_and_by_model_are_ranked_and_labelled() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY_MS;
        seed(
            &db,
            "s1",
            now - 1000,
            "claude-fable-5",
            TurnUsage {
                input: 2_000_000,
                ..Default::default()
            },
        );
        seed(
            &db,
            "s2",
            now - 2000,
            "claude-sonnet-5",
            TurnUsage {
                input: 1_000_000,
                ..Default::default()
            },
        );

        let m = build_usage(&db, now).unwrap();
        assert_eq!(m.top_projects[0].name, "proj");
        assert_eq!(
            m.top_projects[0].tokens, 3_000_000,
            "chart value is a number"
        );
        assert_eq!(
            m.top_projects[0].tokens_label, "3.0M",
            "its label is a finished string"
        );

        let models: Vec<&str> = m.by_model.iter().map(|x| x.model.as_str()).collect();
        assert_eq!(
            models[0], "claude-fable-5",
            "ranked by tokens, biggest first"
        );
        assert_eq!(m.by_model[0].cost, "$30.00");
    }

    #[test]
    fn burn_rate_is_absent_when_the_window_is_too_young_to_project() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        // Usage exists, but the oldest turn inside the window is only one
        // minute old — not enough history to project an hourly rate from.
        let now = 100 * DAY_MS + 300_000;
        seed(
            &db,
            "s1",
            now - 60_000,
            "claude-fable-5",
            TurnUsage {
                input: 1_000,
                ..Default::default()
            },
        );
        let m = build_usage(&db, now).unwrap();
        assert!(
            m.burn_rate.is_none(),
            "one minute of history is not enough to project from"
        );
    }

    #[test]
    fn burn_rate_appears_once_the_window_has_enough_history() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY_MS + 2 * 3_600_000; // two hours into the window
        seed(
            &db,
            "s1",
            now - 3_600_000,
            "claude-fable-5",
            TurnUsage {
                input: 2_000_000,
                ..Default::default()
            },
        );
        let m = build_usage(&db, now).unwrap();
        let rate = m.burn_rate.expect("one hour of history is projectable");
        assert!(rate.contains("/h"), "reads as a rate: {rate}");
        assert!(
            !rate.contains('%'),
            "never a percentage of a ceiling Perch does not know"
        );
    }
}
