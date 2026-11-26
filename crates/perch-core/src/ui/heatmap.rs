//! A year of activity as a calendar grid, one cell per local day, like GitHub's
//! contribution graph. Built for the whole index or for one project, so every
//! view that shows one draws the same thing.

use crate::db::Db;
use crate::query::{self, DayUsage};
use crate::ui::format::{cost_label, plural};
use chrono::{DateTime, Datelike, Days, NaiveDate, TimeZone, Weekday};

/// Columns in the Overview's grid: a year, plus the week in progress.
pub const WEEKS: u32 = 53;
/// Columns on a project's page, which has less to show and less room.
pub const PROJECT_WEEKS: u32 = 26;

fn weeks_for(project_id: Option<i64>) -> u32 {
    if project_id.is_some() {
        PROJECT_WEEKS
    } else {
        WEEKS
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Heatmap {
    /// "Last year" or "Last 26 weeks": the span the grid covers.
    pub title: String,
    pub columns: u32,
    /// Today and every day before it in the grid. Days after today aren't here.
    pub cells: Vec<HeatCell>,
    /// Above the column where a month begins.
    pub months: Vec<AxisLabel>,
    /// Beside every other row, so the column of labels isn't crowded.
    pub weekdays: Vec<AxisLabel>,
    /// "212 active days · current streak 4 days · longest 19 days · busiest Oct 24".
    pub summary: String,
    /// Figures over the grid's span, for a stat row: active days, busiest day.
    pub stats: Vec<HeatStat>,
    /// "Tracking since Jul 19", when records begin inside the grid.
    pub tracking_note: Option<String>,
    pub legend_less: String,
    pub legend_more: String,
    pub legend_untracked: String,
    /// False when the index has no activity at all, here or ever.
    pub has_data: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HeatCell {
    pub column: u32,
    /// 0 is the user's first day of the week.
    pub row: u32,
    /// 0 for a quiet day, 1 to 4 for an active one, relative to the user's own
    /// active days.
    pub level: u8,
    /// Before Perch's records begin, so unknown rather than quiet.
    pub before_tracking: bool,
    /// "Fri, Oct 24: 312 responses, 5 sessions, $18.40".
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HeatStat {
    /// "Active days".
    pub label: String,
    /// "41 of 182", or "Oct 24".
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AxisLabel {
    /// A column for a month, a row for a weekday.
    pub index: u32,
    pub label: String,
}

/// `first_weekday` is the user's first day of the week, from their locale:
/// 1 for Sunday through 7 for Saturday, as macOS numbers them.
pub fn build_heatmap(
    db: &Db,
    project_id: Option<i64>,
    now_ms: i64,
    first_weekday: u8,
    show_cost: bool,
) -> anyhow::Result<Heatmap> {
    build_in(
        db,
        &chrono::Local,
        project_id,
        now_ms,
        first_weekday,
        show_cost,
    )
}

pub(crate) fn build_in<Tz: TimeZone>(
    db: &Db,
    tz: &Tz,
    project_id: Option<i64>,
    now_ms: i64,
    first_weekday: u8,
    show_cost: bool,
) -> anyhow::Result<Heatmap> {
    let local_date = |ms: i64| {
        DateTime::from_timestamp_millis(ms)
            .unwrap_or_default()
            .with_timezone(tz)
            .date_naive()
    };
    let today = local_date(now_ms);
    let weeks = weeks_for(project_id);
    let span_name = if weeks == WEEKS {
        "the last year".to_string()
    } else {
        format!("the last {weeks} weeks")
    };
    let week_start = weekday_from_mac(first_weekday);
    let into_week = today.weekday().days_since(week_start);
    let grid_start = today
        .checked_sub_days(Days::new(u64::from(into_week + (weeks - 1) * 7)))
        .unwrap_or(today);
    let span = (today - grid_start).num_days() as usize + 1;

    let days = query::daily_usage_in(db, tz, span, now_ms, project_id)?;
    let first_tracked = query::first_turn_at(db, project_id)?.map(local_date);
    let tracked = |date: NaiveDate| first_tracked.is_some_and(|first| date >= first);
    let levels = levels(&days.iter().map(|d| d.responses).collect::<Vec<_>>());

    let cells = days
        .iter()
        .zip(&levels)
        .enumerate()
        .map(|(i, (day, &level))| HeatCell {
            column: (i / 7) as u32,
            row: (i % 7) as u32,
            level,
            before_tracking: !tracked(day.date),
            label: cell_label(day, tracked(day.date), today, show_cost),
        })
        .collect();

    Ok(Heatmap {
        title: capitalise(span_name.trim_start_matches("the ")),
        columns: weeks,
        cells,
        months: month_labels(grid_start, span, today),
        weekdays: (0..7u32)
            .filter(|row| row % 2 == 1)
            .map(|row| AxisLabel {
                index: row,
                label: grid_start
                    .checked_add_days(Days::new(u64::from(row)))
                    .unwrap_or(grid_start)
                    .format("%a")
                    .to_string(),
            })
            .collect(),
        summary: summary(&days, today, &span_name),
        stats: stats(&days, today),
        tracking_note: first_tracked
            .filter(|first| *first > grid_start)
            .map(|first| format!("Tracking since {}", date_label(first, today))),
        legend_less: "Less".into(),
        legend_more: "More".into(),
        legend_untracked: "Before tracking".into(),
        has_data: first_tracked.is_some(),
    })
}

/// macOS numbers weekdays from 1 for Sunday. Anything else falls back to Monday.
fn weekday_from_mac(n: u8) -> Weekday {
    match n {
        1 => Weekday::Sun,
        2 => Weekday::Mon,
        3 => Weekday::Tue,
        4 => Weekday::Wed,
        5 => Weekday::Thu,
        6 => Weekday::Fri,
        7 => Weekday::Sat,
        _ => Weekday::Mon,
    }
}

/// Each active day's level comes from where it ranks among the active days,
/// not from its share of the busiest one. One enormous day then can't fade
/// every other day to the lowest shade.
pub(crate) fn levels(responses: &[u64]) -> Vec<u8> {
    let mut active: Vec<u64> = responses.iter().copied().filter(|&n| n > 0).collect();
    active.sort_unstable();
    let total = active.len();
    responses
        .iter()
        .map(|&n| {
            if n == 0 {
                return 0;
            }
            let at_or_below = active.partition_point(|&v| v <= n);
            (4 * at_or_below).div_ceil(total).clamp(1, 4) as u8
        })
        .collect()
}

/// "Oct 24", with the year when it isn't this year's.
pub(crate) fn date_label(date: NaiveDate, today: NaiveDate) -> String {
    if date.year() == today.year() {
        date.format("%b %-d").to_string()
    } else {
        date.format("%b %-d, %Y").to_string()
    }
}

fn cell_label(day: &DayUsage, tracked: bool, today: NaiveDate, show_cost: bool) -> String {
    let date = format!("{}, {}", day.date.format("%a"), date_label(day.date, today));
    if !tracked {
        return format!("{date}: before Perch's records begin");
    }
    if day.responses == 0 {
        return format!("{date}: no activity");
    }
    let mut parts = vec![
        plural(day.responses as i64, "response", "responses"),
        plural(day.sessions as i64, "session", "sessions"),
    ];
    if show_cost {
        parts.push(cost_label(
            day.cost_usd,
            day.unpriced_tokens,
            day.usage.total_tokens(),
        ));
    }
    format!("{date}: {}", parts.join(", "))
}

/// A month's name over the first column holding its 1st. The grid's first
/// column is labelled too, unless the next month's name would crowd it.
fn month_labels(grid_start: NaiveDate, span: usize, today: NaiveDate) -> Vec<AxisLabel> {
    let mut out: Vec<AxisLabel> = Vec::new();
    for i in 0..span {
        let Some(date) = grid_start.checked_add_days(Days::new(i as u64)) else {
            break;
        };
        let column = (i / 7) as u32;
        if date.day() == 1 || i == 0 {
            let label = if date.year() == today.year() || date.month() != 1 {
                date.format("%b").to_string()
            } else {
                date.format("%b %Y").to_string()
            };
            if out.last().is_some_and(|l| l.index == column) {
                out.pop();
            }
            out.push(AxisLabel {
                index: column,
                label,
            });
        }
    }
    // Two names within three columns of each other overlap; the earlier one goes.
    let mut kept: Vec<AxisLabel> = Vec::new();
    for label in out {
        if kept.last().is_some_and(|l| label.index < l.index + 3) {
            kept.pop();
        }
        kept.push(label);
    }
    kept
}

/// The longest run of active days, and the run that is still going: one that
/// ends today, or yesterday while today has nothing yet.
fn streaks(days: &[DayUsage]) -> (usize, usize) {
    let mut longest = 0;
    let mut run = 0;
    for day in days {
        run = if day.responses > 0 { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let mut current = days
        .iter()
        .rev()
        .skip_while(|d| d.responses == 0)
        .take_while(|d| d.responses > 0)
        .count();
    let quiet_tail = days.iter().rev().take_while(|d| d.responses == 0).count();
    if quiet_tail > 1 {
        current = 0;
    }
    (current, longest)
}

/// "Last 26 weeks" from "last 26 weeks".
fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// The day with the most responses; the earliest of equals.
fn busiest(days: &[DayUsage]) -> Option<&DayUsage> {
    days.iter()
        .filter(|d| d.responses > 0)
        .max_by_key(|d| (d.responses, std::cmp::Reverse(d.date)))
}

fn stats(days: &[DayUsage], today: NaiveDate) -> Vec<HeatStat> {
    let active = days.iter().filter(|d| d.responses > 0).count();
    vec![
        HeatStat {
            label: "Active days".into(),
            value: format!("{active} of {}", days.len()),
        },
        HeatStat {
            label: "Busiest day".into(),
            value: busiest(days)
                .map(|d| date_label(d.date, today))
                .unwrap_or_else(|| "—".into()),
        },
    ]
}

fn summary(days: &[DayUsage], today: NaiveDate, span_name: &str) -> String {
    let active = days.iter().filter(|d| d.responses > 0).count();
    if active == 0 {
        return format!("No activity in {span_name}");
    }
    let (current, longest) = streaks(days);
    let busiest = busiest(days)
        .map(|d| date_label(d.date, today))
        .unwrap_or_default();
    format!(
        "{} · current streak {} · longest {} · busiest {busiest}",
        plural(active as i64, "active day", "active days"),
        plural(current as i64, "day", "days"),
        plural(longest as i64, "day", "days"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn, TurnUsage};
    use crate::pricing::seed_default_prices;
    use chrono::Utc;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn noon(d: NaiveDate) -> i64 {
        d.and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    fn day(d: NaiveDate, responses: u64) -> DayUsage {
        DayUsage {
            date: d,
            responses,
            sessions: responses.min(1),
            usage: TurnUsage::default(),
            cost_usd: 0.0,
            unpriced_tokens: 0,
            by_model: Vec::new(),
        }
    }

    fn run(pattern: &[u64]) -> Vec<DayUsage> {
        let start = date(2026, 1, 1);
        pattern
            .iter()
            .enumerate()
            .map(|(i, &n)| day(start + Days::new(i as u64), n))
            .collect()
    }

    /// One turn per entry, each in its own session, at noon UTC.
    fn seed(db: &Db, turns: &[(NaiveDate, &str)]) -> i64 {
        let pid = db.upsert_project("-a-p", "/a/p", false).unwrap();
        for (i, (d, model)) in turns.iter().enumerate() {
            let id = format!("s{i}");
            db.upsert_session(&SessionRecord {
                id: id.clone(),
                project_id: pid,
                file_path: format!("/tmp/{id}.jsonl"),
                file_size: 0,
                indexed_offset: 0,
                started_at: Some(noon(*d)),
                last_activity_at: Some(noon(*d)),
                cwd: Some("/a/p".into()),
                git_branch: None,
                cc_version: None,
                title: None,
                message_count: 1,
            })
            .unwrap();
            db.insert_turns(
                &id,
                &[Turn {
                    message_id: None,
                    ts: noon(*d),
                    model: (*model).into(),
                    usage: TurnUsage {
                        output: 1_000_000,
                        ..TurnUsage::default()
                    },
                }],
            )
            .unwrap();
        }
        pid
    }

    #[test]
    fn levels_rank_active_days_so_one_huge_day_does_not_fade_the_rest() {
        assert_eq!(levels(&[0, 1, 2, 3, 4]), vec![0, 1, 2, 3, 4]);
        assert_eq!(
            levels(&[5, 6, 7, 10_000]),
            vec![1, 2, 3, 4],
            "the ordinary days still spread across the shades"
        );
        assert_eq!(levels(&[3, 3, 3]), vec![4, 4, 4], "equal days look equal");
        assert_eq!(levels(&[0, 0]), vec![0, 0]);
        assert_eq!(
            levels(&[0, 9]),
            vec![0, 4],
            "a lone active day is fully lit"
        );
    }

    #[test]
    fn a_streak_still_counts_while_today_is_quiet_so_far() {
        assert_eq!(streaks(&run(&[1, 1, 0, 1, 1, 1])), (3, 3));
        assert_eq!(
            streaks(&run(&[1, 1, 1, 0, 1, 1, 0])),
            (2, 3),
            "today has nothing yet, so yesterday's run continues"
        );
        assert_eq!(
            streaks(&run(&[1, 1, 1, 1, 0, 0])),
            (0, 4),
            "two quiet days end it"
        );
        assert_eq!(streaks(&run(&[0, 0, 0])), (0, 0));
    }

    #[test]
    fn the_summary_names_active_days_streaks_and_the_busiest_day() {
        let days = run(&[2, 9, 0, 9, 1]);
        assert_eq!(
            summary(&days, date(2026, 6, 1), "the last year"),
            "4 active days · current streak 2 days · longest 2 days · busiest Jan 2"
        );
        assert_eq!(
            summary(&run(&[0, 0]), date(2026, 6, 1), "the last 26 weeks"),
            "No activity in the last 26 weeks"
        );
    }

    #[test]
    fn the_stat_row_counts_active_days_and_names_the_busiest() {
        let s = stats(&run(&[2, 9, 0, 9, 1]), date(2026, 6, 1));
        assert_eq!(s[0].label, "Active days");
        assert_eq!(s[0].value, "4 of 5");
        assert_eq!(s[1].label, "Busiest day");
        assert_eq!(s[1].value, "Jan 2", "the earlier of equals");
        let quiet = stats(&run(&[0, 0]), date(2026, 6, 1));
        assert_eq!(quiet[0].value, "0 of 2");
        assert_eq!(quiet[1].value, "—");
    }

    #[test]
    fn a_project_spans_26_weeks_and_the_overview_a_year() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let today = date(2026, 11, 6);
        seed(&db, &[(today, "claude-sonnet-5")]);
        let pid: i64 = db
            .conn()
            .query_row("SELECT id FROM projects LIMIT 1", [], |r| r.get(0))
            .unwrap();

        let all = build_in(&db, &Utc, None, noon(today), 2, true).unwrap();
        assert_eq!((all.columns, all.title.as_str()), (53, "Last year"));
        let one = build_in(&db, &Utc, Some(pid), noon(today), 2, true).unwrap();
        assert_eq!((one.columns, one.title.as_str()), (26, "Last 26 weeks"));
        assert!(one.cells.iter().all(|c| c.column < 26));
        assert_eq!(
            one.cells.last().unwrap().column,
            25,
            "today is in the last column"
        );
        assert!(one.has_data);
        assert_eq!(one.stats[0].value, format!("1 of {}", one.cells.len()));
    }

    #[test]
    fn the_grid_ends_today_and_its_columns_start_on_the_users_first_weekday() {
        let db = open_in_memory().unwrap();
        let today = date(2026, 11, 6); // a Friday
        for (first_weekday, today_row) in [(1u8, 5u32), (2, 4), (7, 6)] {
            let h = build_in(&db, &Utc, None, noon(today), first_weekday, true).unwrap();
            let last = h.cells.last().unwrap();
            assert_eq!(last.column, WEEKS - 1, "today is in the last column");
            assert_eq!(last.row, today_row, "first weekday {first_weekday}");
            assert!(last.label.starts_with("Fri, Nov 6"), "{}", last.label);
            assert_eq!(h.cells.len() as u32, (WEEKS - 1) * 7 + today_row + 1);
            assert!(
                h.cells[0].label.contains("2025"),
                "last year's dates say so"
            );
        }
    }

    #[test]
    fn days_before_records_begin_look_different_from_quiet_days() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let today = date(2026, 11, 6);
        seed(&db, &[(date(2026, 7, 19), "claude-sonnet-5")]);

        let h = build_in(&db, &Utc, None, noon(today), 2, true).unwrap();
        let cell = |d: NaiveDate| {
            let i = (d - date(2026, 11, 6)).num_days() + h.cells.len() as i64 - 1;
            h.cells[i as usize].clone()
        };
        assert!(cell(date(2026, 7, 18)).before_tracking);
        assert!(cell(date(2026, 7, 18))
            .label
            .ends_with("before Perch's records begin"));
        let first = cell(date(2026, 7, 19));
        assert!(!first.before_tracking);
        assert_eq!(first.level, 4);
        assert!(
            first
                .label
                .starts_with("Sun, Jul 19: 1 response, 1 session, $"),
            "{}",
            first.label
        );
        let quiet = cell(date(2026, 7, 20));
        assert!(!quiet.before_tracking);
        assert_eq!(quiet.level, 0);
        assert!(quiet.label.ends_with(": no activity"));
        assert_eq!(h.tracking_note.as_deref(), Some("Tracking since Jul 19"));
        assert!(h.has_data);
    }

    #[test]
    fn an_unpriced_day_says_so_and_hidden_costs_stay_hidden() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let today = date(2026, 11, 6);
        seed(&db, &[(today, "claude-future-1")]);

        let shown = build_in(&db, &Utc, None, noon(today), 2, true).unwrap();
        assert!(shown.cells.last().unwrap().label.ends_with(", no price"));
        let hidden = build_in(&db, &Utc, None, noon(today), 2, false).unwrap();
        assert!(hidden.cells.last().unwrap().label.ends_with(", 1 session"));
    }

    #[test]
    fn one_project_shows_only_its_own_days() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let today = date(2026, 11, 6);
        seed(&db, &[(today, "claude-sonnet-5")]);
        let other = db.upsert_project("-a-o", "/a/o", false).unwrap();

        let h = build_in(&db, &Utc, Some(other), noon(today), 2, true).unwrap();
        assert!(!h.has_data);
        assert!(h.cells.iter().all(|c| c.level == 0 && c.before_tracking));
    }

    #[test]
    fn month_names_sit_over_the_column_where_the_month_begins() {
        let db = open_in_memory().unwrap();
        let h = build_in(&db, &Utc, None, noon(date(2026, 11, 6)), 2, true).unwrap();
        let names: Vec<&str> = h.months.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(names.first(), Some(&"Nov"), "{names:?}");
        assert_eq!(names.last(), Some(&"Nov"), "{names:?}");
        assert!(names.contains(&"Jan"), "{names:?}");
        assert!(h.months.windows(2).all(|w| w[1].index >= w[0].index + 3));
        let weekdays: Vec<&str> = h.weekdays.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(weekdays, vec!["Tue", "Thu", "Sat"]);
    }
}
