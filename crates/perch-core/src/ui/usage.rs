//! The usage view's view-model. Chart values cross as numbers; every label is
//! finished here.

use crate::db::Db;
use crate::query::{self, DAY_MS};
use crate::settings::{BurnRate, Settings};
use crate::ui::format::{human_cost, human_tokens, plural};

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
    pub tokens: u64,
    pub tokens_label: String,
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

/// "1 token", "999 tokens", "2.0M tokens". `plural` owns the singular/plural
/// spelling and `human_tokens` owns the compact one; below a thousand they
/// agree on the digits, and above it nothing is ever singular.
fn tokens_phrase(n: u64) -> String {
    if n < 1_000 {
        plural(n as i64, "token", "tokens")
    } else {
        format!("{} tokens", human_tokens(n))
    }
}

/// The burn-rate line in whichever unit the user asked for, or `None` when the
/// window is too young or too quiet to project from honestly. Deliberately not
/// a percentage: the ceiling needs the tier-1 usage endpoint, and the original
/// spec §8 forbids inventing one.
///
/// `MIN_PROJECTABLE_MS` gates *every* mode, before the mode is even consulted.
/// It is a statistical-soundness floor, not a preference: a rate that appeared
/// in one unit but not another, for identical data, would be a lie about what
/// Perch knows.
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
fn burn_rate(
    mode: BurnRate,
    show_cost: bool,
    window_tokens: u64,
    window_cost: f64,
    elapsed_ms: i64,
) -> Option<String> {
    if mode == BurnRate::Off || window_tokens == 0 || elapsed_ms < MIN_PROJECTABLE_MS {
        return None;
    }
    let hours = elapsed_ms as f64 / 3_600_000.0;

    // A dollar-denominated mode with dollars switched off falls back to
    // tokens rather than to silence. Silence means one specific thing in this
    // view — "not enough data in this window to project a rate", which is
    // what the settings preview says in as many words — and saying that of a
    // window full of data would be a lie in the opposite direction from the
    // one `MIN_PROJECTABLE_MS` guards against. The pace is known; only the
    // currency is unwelcome.
    let mode = match (mode, show_cost) {
        (BurnRate::CostPerHour | BurnRate::CostPerDay, false) => BurnRate::TokensPerHour,
        (m, _) => m,
    };

    Some(match mode {
        // Returned above; repeated only to keep the match exhaustive, so a
        // sixth mode is a compile error here rather than a silent default.
        BurnRate::Off => return None,
        BurnRate::TokensPerHour => format!(
            "≈{} per hour at this pace",
            tokens_phrase((window_tokens as f64 / hours).round() as u64)
        ),
        BurnRate::CostPerHour => format!("≈{}/hr at this pace", human_cost(window_cost / hours)),
        BurnRate::CostPerDay => {
            format!(
                "≈{}/day at this pace",
                human_cost(window_cost / hours * 24.0)
            )
        }
        // The whole window's total at this pace, not a rate — and still only
        // ever offered from a window long enough to project from.
        BurnRate::ProjectedWindow => format!(
            "≈{} by the end of this window",
            tokens_phrase(
                (window_tokens as f64 * (WINDOW_MS as f64 / elapsed_ms as f64)).round() as u64
            )
        ),
    })
}

/// How long ago the oldest observed turn happened, relative to `now_ms` — i.e.
/// how much of the trailing window Perch actually has turns for. Zero when
/// the window is empty. The database access itself lives in
/// `query::oldest_turn_since`; this just turns that timestamp into a duration.
fn observed_elapsed_ms(oldest_ts: Option<i64>, now_ms: i64) -> i64 {
    match oldest_ts {
        Some(ts) => now_ms - ts,
        None => 0,
    }
}

/// `settings.chart_days` spans the daily chart — the same field
/// `build_project_detail`'s sparkline uses, so the two can never disagree
/// about their own date range — while `top_projects_count` and
/// `top_projects_days` shape the ranked list.
pub fn build_usage(db: &Db, now_ms: i64, settings: &Settings) -> anyhow::Result<UsageModel> {
    let window_since = now_ms - WINDOW_MS;
    let (window, window_cost) = query::usage_since(db, window_since)?;
    let (day, day_cost) = query::usage_since(db, now_ms - DAY_MS)?;
    let (week, _) = query::usage_since(db, now_ms - 7 * DAY_MS)?;

    // `show_cost` off means absent, not "$0.00": a fabricated zero would read
    // as a priced model that cost nothing.
    let shown_cost = |usd: f64| {
        if settings.show_cost {
            human_cost(usd)
        } else {
            String::new()
        }
    };

    let chart_days = settings.chart_days as usize;
    let days = query::daily_usage(db, chart_days, now_ms)?;
    let daily: Vec<DailyBar> = days
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let days_ago = chart_days - 1 - i;
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

    let top_projects = query::top_projects(
        db,
        now_ms - i64::from(settings.top_projects_days) * DAY_MS,
        settings.top_projects_count as usize,
    )?
    .into_iter()
    .map(|(name, u, cost)| RankedProject {
        name,
        tokens: u.total_tokens(),
        tokens_label: human_tokens(u.total_tokens()),
        cost: shown_cost(cost),
    })
    .collect();

    let mut by_model_rows = query::usage_by_model(db)?;
    // `usage_by_model` is unordered by omission (no `ORDER BY` in its SQL), so
    // rank here — ties broken by model name, the same way `top_projects`
    // breaks ties, so two models with identical totals still render in a
    // deterministic order rather than whatever SQLite happened to return.
    by_model_rows.sort_by(|a, b| {
        b.1.total_tokens()
            .cmp(&a.1.total_tokens())
            .then_with(|| a.0.cmp(&b.0))
    });
    let by_model = by_model_rows
        .into_iter()
        .map(|(model, u, cost)| ModelUsage {
            model,
            tokens: u.total_tokens(),
            tokens_label: human_tokens(u.total_tokens()),
            cost: shown_cost(cost),
        })
        .collect();

    let elapsed_in_window =
        observed_elapsed_ms(query::oldest_turn_since(db, window_since)?, now_ms);

    let hero = vec![
        HeroStat {
            label: "5-hour window".into(),
            value: human_tokens(window.total_tokens()),
            caption: settings.show_cost.then(|| human_cost(window_cost)),
        },
        HeroStat {
            label: "Week".into(),
            value: human_tokens(week.total_tokens()),
            caption: None,
        },
        HeroStat {
            label: "24 hours".into(),
            value: human_tokens(day.total_tokens()),
            caption: settings.show_cost.then(|| human_cost(day_cost)),
        },
    ];

    Ok(UsageModel {
        has_data: db.turn_count()? > 0,
        burn_rate: burn_rate(
            settings.burn_rate,
            settings.show_cost,
            window.total_tokens(),
            window_cost,
            elapsed_in_window,
        ),
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
            title: None,
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
        let m = build_usage(&db, 100 * DAY_MS, &Settings::default()).unwrap();
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

        let m = build_usage(&db, now, &Settings::default()).unwrap();
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

        let m = build_usage(&db, now, &Settings::default()).unwrap();
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
        assert_eq!(m.by_model[0].tokens, 2_000_000, "chart value is a number");
        assert_eq!(m.by_model[0].tokens_label, "2.0M");
        assert_eq!(m.by_model[0].cost, "$30.00");
    }

    /// This scenario is deliberately built so the rejected `rem_euclid`-based
    /// elapsed time and the oldest-observed-turn elapsed time land on
    /// *opposite* sides of the 30-minute floor: `now` is chosen so
    /// `now.rem_euclid(WINDOW_MS)` is a comfortable 2 hours (which the old,
    /// epoch-assuming basis would treat as plenty of history), while the only
    /// turn Perch has actually seen is 5 minutes old. If the implementation
    /// ever regresses to the epoch-aligned basis, this turns `Some(..)` and
    /// the assertion below fails — a same-side-of-the-floor scenario (as the
    /// original brief's) would not have caught that.
    #[test]
    fn burn_rate_is_absent_when_the_window_is_too_young_to_project() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 7_200_000; // now.rem_euclid(WINDOW_MS) == 7_200_000 (2h)
        seed(
            &db,
            "s1",
            now - 300_000, // oldest (only) turn is 5 minutes old
            "claude-fable-5",
            TurnUsage {
                input: 1_000,
                ..Default::default()
            },
        );
        let m = build_usage(&db, now, &Settings::default()).unwrap();
        assert!(
            m.burn_rate.is_none(),
            "the oldest turn Perch has seen in this window is 5 minutes old \
             (well under the 30-minute floor), even though now.rem_euclid(WINDOW_MS) \
             is 2 hours — proving elapsed time is measured from observed history, \
             not from an epoch-aligned boundary"
        );
    }

    /// Mirrors the test above in the other direction: `now` is chosen so
    /// `now.rem_euclid(WINDOW_MS)` is only 10 minutes (which the old basis
    /// would wrongly call too young to project), while the turn Perch
    /// actually observed is a full hour old. The exact projected rate is
    /// asserted too, not just its shape, so a regression to the rejected
    /// basis (which would compute a different elapsed time, or `None`) is
    /// caught even if the shape-only checks would have passed.
    #[test]
    fn burn_rate_appears_once_the_window_has_enough_history() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 600_000; // now.rem_euclid(WINDOW_MS) == 600_000 (10m)
        seed(
            &db,
            "s1",
            now - 3_600_000, // oldest (only) turn is 1 hour old
            "claude-fable-5",
            TurnUsage {
                input: 2_000_000,
                ..Default::default()
            },
        );
        // Tokens per hour, named explicitly: this test pins an elapsed-time
        // *basis*, so it wants the unit that shows that basis's arithmetic
        // directly rather than whichever unit happens to be the default.
        let s = Settings {
            burn_rate: BurnRate::TokensPerHour,
            ..Settings::default()
        };
        let m = build_usage(&db, now, &s).unwrap();
        let rate = m.burn_rate.expect(
            "the oldest turn Perch has seen in this window is 1 hour old \
             (over the 30-minute floor), even though now.rem_euclid(WINDOW_MS) \
             is only 10 minutes",
        );
        // 2,000,000 tokens observed over exactly 1 hour of history projects
        // to exactly 2,000,000/h — pinning the actual figure, not just that
        // it reads as a rate.
        assert_eq!(rate, "≈2.0M tokens per hour at this pace");
        assert!(
            !rate.contains('%'),
            "never a percentage of a ceiling Perch does not know"
        );
    }

    /// A window with plenty of history and plenty of tokens: every burn-rate
    /// mode has something to say about it.
    fn busy_db() -> (Db, i64) {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 600_000;
        seed(
            &db,
            "s1",
            now - 3_600_000, // an hour of observed history
            "claude-fable-5",
            TurnUsage {
                input: 2_000_000, // $15/Mtok → $30.00 over that hour
                ..Default::default()
            },
        );
        (db, now)
    }

    /// The same tokens, but only ten minutes of observed history — under
    /// `MIN_PROJECTABLE_MS`, so there is nothing honest to project from.
    fn thin_db() -> (Db, i64) {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 600_000;
        seed(
            &db,
            "s1",
            now - 600_000,
            "claude-fable-5",
            TurnUsage {
                input: 2_000_000,
                ..Default::default()
            },
        );
        (db, now)
    }

    #[test]
    fn burn_rate_off_produces_no_line_at_all() {
        let (db, now) = busy_db();
        let s = Settings {
            burn_rate: BurnRate::Off,
            ..Settings::default()
        };
        assert!(build_usage(&db, now, &s).unwrap().burn_rate.is_none());
    }

    #[test]
    fn each_burn_rate_mode_names_its_own_unit() {
        let (db, now) = busy_db();
        let cases = [
            (BurnRate::TokensPerHour, "per hour"),
            (BurnRate::CostPerHour, "/hr"),
            (BurnRate::CostPerDay, "/day"),
            (BurnRate::ProjectedWindow, "by the end of this window"),
        ];
        for (mode, expected) in cases {
            let s = Settings {
                burn_rate: mode,
                ..Settings::default()
            };
            let line = build_usage(&db, now, &s)
                .unwrap()
                .burn_rate
                .expect("an hour of history and two million tokens is enough");
            assert!(line.contains(expected), "{mode:?} produced {line:?}");
        }
    }

    #[test]
    fn the_figures_each_mode_reports_are_the_ones_the_window_earns() {
        let (db, now) = busy_db();
        // 2,000,000 tokens at $15/Mtok over exactly one observed hour.
        let cases = [
            (
                BurnRate::TokensPerHour,
                "≈2.0M tokens per hour at this pace",
            ),
            (BurnRate::CostPerHour, "≈$30.00/hr at this pace"),
            (BurnRate::CostPerDay, "≈$720.00/day at this pace"),
            (
                BurnRate::ProjectedWindow,
                "≈10.0M tokens by the end of this window",
            ),
        ];
        for (mode, expected) in cases {
            let s = Settings {
                burn_rate: mode,
                ..Settings::default()
            };
            assert_eq!(
                build_usage(&db, now, &s).unwrap().burn_rate.as_deref(),
                Some(expected),
                "{mode:?}"
            );
        }
    }

    /// `MIN_PROJECTABLE_MS` is a statistical floor, not a preference: no
    /// display mode may talk a projection out of a window too short to
    /// support one. A rate that appeared in one unit but not another, for
    /// identical data, would be a lie about what Perch knows.
    #[test]
    fn a_thin_window_still_suppresses_every_mode() {
        let (db, now) = thin_db();
        for mode in [
            BurnRate::TokensPerHour,
            BurnRate::CostPerHour,
            BurnRate::CostPerDay,
            BurnRate::ProjectedWindow,
        ] {
            let s = Settings {
                burn_rate: mode,
                ..Settings::default()
            };
            assert!(
                build_usage(&db, now, &s).unwrap().burn_rate.is_none(),
                "{mode:?} projected from ten minutes of history"
            );
        }
    }

    #[test]
    fn hiding_cost_hides_it_everywhere_it_would_appear() {
        let (db, now) = busy_db();
        let s = Settings {
            show_cost: false,
            ..Settings::default() // whose burn_rate is a *cost* per hour
        };
        let m = build_usage(&db, now, &s).unwrap();
        assert!(m.hero.iter().all(|h| !h.value.contains('$')));
        assert!(
            m.hero.iter().all(|h| h.caption.is_none()),
            "the hero captions are the costs; hidden means absent, not $0.00"
        );
        assert_eq!(m.burn_rate.as_deref().unwrap_or("").matches('$').count(), 0);
        assert!(m.top_projects.iter().all(|p| !p.cost.contains('$')));
        assert!(m.by_model.iter().all(|r| !r.cost.contains('$')));
    }

    #[test]
    fn a_cost_burn_rate_with_cost_hidden_still_reports_the_pace() {
        let (db, now) = busy_db();
        let s = Settings {
            show_cost: false,
            burn_rate: BurnRate::CostPerHour,
            ..Settings::default()
        };
        // Silence would read as "not enough data to project" — which is what
        // the settings preview says a missing line means. The pace is known;
        // only the currency is unwelcome.
        assert_eq!(
            build_usage(&db, now, &s).unwrap().burn_rate.as_deref(),
            Some("≈2.0M tokens per hour at this pace")
        );
    }

    #[test]
    fn showing_cost_still_shows_it() {
        let (db, now) = busy_db();
        let m = build_usage(&db, now, &Settings::default()).unwrap();
        assert_eq!(m.hero[0].caption.as_deref(), Some("$30.00"));
        assert_eq!(m.top_projects[0].cost, "$30.00");
        assert_eq!(m.by_model[0].cost, "$30.00");
    }
}
