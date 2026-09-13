//! The view-model every shell renders. All strings are final here.

use crate::db::Db;
use crate::live::{LiveSession, SessionStatus};
use crate::settings::{MenuBarDisplay, MenuBarIcon, RowDensity, Settings};
use crate::ui::format::{elapsed_or_dash, human_cost, human_elapsed, human_tokens};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Status {
    Working,
    Idle,
    Waiting,
    Background,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Stats {
    pub window_tokens: String,
    pub week_tokens: String,
    pub day_tokens: String,
    pub day_cost: String,
    /// Always true until the tier-1 usage endpoint lands (spec §8).
    pub estimated: bool,
    pub has_data: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SessionRow {
    pub id: String,
    pub pid: i32,
    pub name: String,
    pub project: String,
    pub kind: String,
    pub version: String,
    pub status: Status,
    pub status_label: String,
    pub elapsed: String,
    /// Empty when `show_row_usage` is off — the user asked not to see this,
    /// which is a different thing from the dash that means "not known".
    pub tokens: String,
    /// Empty when either `show_row_usage` or `show_cost` is off, for the same
    /// reason `tokens` is.
    pub cost: String,
    /// The session's working directory, or empty when `show_row_folder` is
    /// off. `project` is only its last component, which two checkouts can
    /// share; this is the whole path, for the shell to draw under the name.
    pub folder: String,
    /// The composed "project · kind · vVERSION · TOKENS · COST" line, omitting
    /// whichever parts are absent or hidden — a shell renders this verbatim
    /// rather than assembling it (and rather than comparing `tokens`/`version`
    /// against a sentinel, or reading settings, to decide what to omit; those
    /// are Rust's decisions to make). `project`, `kind`, `version`, `tokens`,
    /// and `cost` stay on the row too, for a shell that wants the parts
    /// separately — and are blanked the same way, so the two can never
    /// disagree about what is on show.
    pub detail_line: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RecentRow {
    pub id: String,
    pub name: String,
    pub project: String,
    pub ended_ago: String,
    pub tokens: String,
    /// The composed "TOKENS · ended AGO ago" (or, with no usage, just "ended
    /// AGO ago") line — same rationale as `SessionRow::detail_line`.
    pub ended_line: String,
}

/// What a shell needs in order to dim honestly: a value whose presence is the
/// flag it acts on, carrying a finished sentence saying how old the data is.
/// Composed here, like every other string, so no shell has to decide what
/// "9m" means — or re-derive the threshold it was measured against.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Staleness {
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PopoverModel {
    pub stats: Stats,
    pub live: Vec<SessionRow>,
    pub recent: Vec<RecentRow>,
    pub tray_title: String,
    pub error: Option<String>,
    /// The finished "N sessions are waiting on you" sentence, or `None` when
    /// nothing is waiting. A shell renders this verbatim — it must not count
    /// or pluralise itself (spec: Rust owns everything except drawing).
    pub waiting_banner: Option<String>,
    /// `Some` once the index has not been read successfully for longer than
    /// `stale_after_minutes`, and never when `dim_when_stale` is off.
    pub staleness: Option<Staleness>,
    /// Which popover sections the user wants drawn. These are fields rather
    /// than something a shell reads out of `Settings` for itself: two readers
    /// of the same preference is exactly how a window and a model come to
    /// disagree about what is on screen.
    pub show_waiting: bool,
    pub show_working: bool,
    /// When false, `recent` is empty as well — a hidden section is not
    /// queried, let alone drawn.
    pub show_recent: bool,
    /// How much breathing room each row gets. Drawing is the shell's; the
    /// choice is not.
    pub row_density: RowDensity,
    /// Which glyph the menu bar item draws. The variant crosses, never a
    /// symbol name: what an SF Symbol is called is the one genuinely
    /// macOS-specific fact here, and it belongs in the macOS shell.
    pub menu_bar_icon: MenuBarIcon,
}

const DASH: &str = "—";
const FIVE_HOURS_MS: i64 = 5 * 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const WEEK_MS: i64 = 7 * DAY_MS;

fn project_of(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string())
}

/// `SessionStatus` (live.rs) has Working | Idle | Waiting | Background | Ended.
/// Blocked wins over everything; a `bg` *kind* renders as Background even when
/// its status says busy, because the user cannot type into it either way.
fn status_of(s: &LiveSession) -> (Status, String, i64) {
    match &s.status {
        SessionStatus::Waiting { reason, since_ms } => (
            Status::Waiting,
            format!("waiting · {}", reason.as_deref().unwrap_or("unknown")),
            *since_ms,
        ),
        SessionStatus::Background => (Status::Background, "background".into(), s.status_updated_at),
        _ if s.kind == "bg" => (Status::Background, "background".into(), s.status_updated_at),
        SessionStatus::Idle => (Status::Idle, "idle".into(), s.status_updated_at),
        SessionStatus::Ended => (Status::Idle, "ended".into(), s.status_updated_at),
        SessionStatus::Working => (Status::Working, "working".into(), s.status_updated_at),
    }
}

/// How old the data on screen is, as a finished sentence, or `None` when
/// there is nothing to say: the user switched dimming off, nothing has ever
/// been read (in which case `PopoverModel::error` carries the more specific
/// story), or the last read is still inside the user's threshold.
///
/// `last_read_ms` is passed in rather than read from a clock here so this
/// stays a pure function of its inputs — the whole model is testable at a
/// fixed instant, and the one place that knows when a read last succeeded is
/// the one place that performs reads.
fn staleness(settings: &Settings, last_read_ms: Option<i64>, now_ms: i64) -> Option<Staleness> {
    if !settings.dim_when_stale {
        return None;
    }
    let gap = now_ms - last_read_ms?;
    if gap <= i64::from(settings.stale_after_minutes) * 60_000 {
        return None;
    }
    // `human_elapsed` caps at hours on purpose: an index Perch has not managed
    // to read for two days should read as 48h, which is more alarming than 2d.
    Some(Staleness {
        label: format!("Last updated {} ago", human_elapsed(gap)),
    })
}

/// The waiting-sessions banner sentence, or `None` when nothing is waiting.
/// Pluralisation lives here so no shell has to re-derive it from a count.
fn waiting_banner(waiting: usize) -> Option<String> {
    match waiting {
        0 => None,
        1 => Some("1 session is waiting on you".to_string()),
        n => Some(format!("{n} sessions are waiting on you")),
    }
}

/// Menu-bar text, shaped by the user's `menu_bar_display` setting:
/// - `Icon`: nothing — the tray icon carries the whole story.
/// - `Count`: the live session count, full stop; never the hourglass, even
///   while something is waiting.
/// - `CountAndWaiting`: the live count, except a blocked count with an
///   hourglass takes over whenever anything is waiting.
pub fn tray_title(live: &[LiveSession], display: MenuBarDisplay) -> String {
    if display == MenuBarDisplay::Icon {
        return String::new();
    }
    if display == MenuBarDisplay::CountAndWaiting {
        let waiting = live
            .iter()
            .filter(|s| matches!(s.status, SessionStatus::Waiting { .. }))
            .count();
        if waiting > 0 {
            return format!("{waiting} ⏳");
        }
    }
    if live.is_empty() {
        String::new()
    } else {
        live.len().to_string()
    }
}

/// Shared with `main_window`, which needs the same "nothing yet" `Stats` for
/// its own no-data states — hence `pub(crate)` rather than private.
pub(crate) fn dashed_stats() -> Stats {
    Stats {
        window_tokens: DASH.into(),
        week_tokens: DASH.into(),
        day_tokens: DASH.into(),
        day_cost: DASH.into(),
        estimated: true,
        has_data: false,
    }
}

impl PopoverModel {
    /// A model with nothing in it — the "before the first tick" state, and what
    /// the window shows for `Now` when there is no engine data yet.
    pub fn empty() -> Self {
        // Nothing has been read yet, so nothing has aged; the visibility and
        // density fields take the shipped defaults rather than inventing a
        // second set of them here.
        let defaults = Settings::default();
        PopoverModel {
            stats: dashed_stats(),
            live: Vec::new(),
            recent: Vec::new(),
            tray_title: String::new(),
            error: None,
            waiting_banner: None,
            staleness: None,
            show_waiting: defaults.show_waiting,
            show_working: defaults.show_working,
            show_recent: defaults.show_recent,
            row_density: defaults.row_density,
            menu_bar_icon: defaults.menu_bar_icon,
        }
    }
}

/// `db.rs` uses `anyhow::Result` throughout (no local `Result` alias), so this
/// does too — the caller in `build_model` collapses any error to a dash.
fn stats_from(db: &Db, now_ms: i64, show_cost: bool) -> anyhow::Result<Stats> {
    use crate::query::usage_since;
    if db.turn_count()? == 0 {
        return Ok(dashed_stats());
    }
    let (w, _) = usage_since(db, now_ms - FIVE_HOURS_MS)?;
    let (k, _) = usage_since(db, now_ms - WEEK_MS)?;
    let (d, d_cost) = usage_since(db, now_ms - DAY_MS)?;
    Ok(Stats {
        window_tokens: human_tokens(w.total_tokens()),
        week_tokens: human_tokens(k.total_tokens()),
        day_tokens: human_tokens(d.total_tokens()),
        // Hidden means absent, not "$0.00" — a fabricated zero is the one
        // thing this model never emits.
        day_cost: if show_cost {
            human_cost(d_cost)
        } else {
            String::new()
        },
        estimated: true,
        has_data: true,
    })
}

/// Build the whole view-model. `db: None` (or a failing db) degrades to sessions-only:
/// the sessions list needs no index, and an honest dash beats a fabricated zero.
///
/// `settings` replaces the bare `MenuBarDisplay` this took before: most of the
/// user's display choices reach this model now — `menu_bar_display`, which
/// shapes `tray_title` alone (see its own doc comment); `recent_limit`, which
/// is how many ended sessions the Recent section lists; the three section
/// toggles and `row_density`, which the model reports so no shell reads them
/// for itself; `show_row_folder`, `show_row_usage` and `show_cost`, which
/// decide what a row says; and `dim_when_stale`/`stale_after_minutes`, which
/// decide whether `staleness` is `Some`. Passing the whole `Settings` rather
/// than the individual values keeps one source of truth for all of them, and
/// is what `build_project_detail` already does.
///
/// `last_read_ms` is when the index was last read successfully — `None`
/// before that has ever happened. It is a parameter rather than a clock
/// reading so this function stays pure and its tests stay deterministic; the
/// caller that performs the reads is the only thing that knows the answer.
pub fn build_model(
    db: Option<&Db>,
    live: &[LiveSession],
    now_ms: i64,
    last_read_ms: Option<i64>,
    settings: &Settings,
) -> PopoverModel {
    let mut error = None;

    let stats = match db.map(|d| stats_from(d, now_ms, settings.show_cost)) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            error = Some(format!("index unavailable: {e}"));
            dashed_stats()
        }
        None => dashed_stats(),
    };

    // Looked up once for the whole tick rather than per row. A failure here
    // is not worth flagging: every row already falls back to the live
    // record's own name, so the worst case is the slug the popover showed
    // before titles existed at all.
    let titles = db
        .and_then(|d| {
            let ids: Vec<&str> = live.iter().map(|s| s.session_id.as_str()).collect();
            crate::query::titles_for(d, &ids).ok()
        })
        .unwrap_or_default();

    // A row- or recent-list-level query failure still degrades honestly (a dash,
    // an empty list — never a fabricated zero), but must not vanish silently:
    // flag it here and fold it into `error` below, without clobbering a more
    // specific message `stats_from` may already have set.
    let mut data_error = false;

    let rows: Vec<SessionRow> = live
        .iter()
        .map(|s| {
            let (status, status_label, since) = status_of(s);
            let (mut tokens, mut cost) =
                match db.map(|d| crate::query::session_usage(d, &s.session_id)) {
                    Some(Ok((u, c))) if u.total_tokens() > 0 => {
                        (human_tokens(u.total_tokens()), human_cost(c))
                    }
                    Some(Ok(_)) => (DASH.into(), DASH.into()),
                    Some(Err(_)) => {
                        data_error = true;
                        (DASH.into(), DASH.into())
                    }
                    None => (DASH.into(), DASH.into()),
                };
            // Two separate preferences, applied in order: hiding usage takes
            // the whole segment, hiding cost takes only the dollars. Both are
            // blanked rather than dashed — an empty string says "you asked
            // not to see this", where a dash says "Perch does not know".
            if !settings.show_row_usage {
                tokens = String::new();
                cost = String::new();
            } else if !settings.show_cost {
                cost = String::new();
            }
            let project = project_of(&s.cwd);
            let kind = s.kind.clone();
            let version = s.cc_version.clone().unwrap_or_default();
            let usage_part = match (tokens.as_str(), cost.as_str()) {
                ("", _) | (DASH, _) => None,
                (t, "") => Some(t.to_string()),
                (t, c) => Some(format!("{t} · {c}")),
            };
            let detail_line = [
                Some(project.clone()),
                Some(kind.clone()),
                (!version.is_empty()).then(|| format!("v{version}")),
                usage_part,
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            SessionRow {
                id: s.session_id.clone(),
                pid: s.pid,
                // The indexed title first: a live record carries Claude
                // Code's `<project>-<id>` slug, which is not what a human
                // named this work. The slug is the fallback, and the id
                // prefix the fallback's fallback.
                name: titles.get(&s.session_id).cloned().unwrap_or_else(|| {
                    if s.name.is_empty() {
                        s.session_id.chars().take(8).collect()
                    } else {
                        s.name.clone()
                    }
                }),
                project,
                kind,
                version,
                status,
                status_label,
                elapsed: elapsed_or_dash(now_ms, since),
                tokens,
                cost,
                folder: if settings.show_row_folder {
                    s.cwd.clone()
                } else {
                    String::new()
                },
                detail_line,
            }
        })
        .collect();

    let live_ids: Vec<String> = live.iter().map(|s| s.session_id.clone()).collect();
    let recent: Vec<RecentRow> =
        // `recent_sessions` returns nothing for a limit of zero (its own explicit
        // guard, kept deliberately); `recent_limit` is bounded 1–20 by
        // `Settings::validated`, so zero cannot arrive from a settings file.
        //
        // `show_recent` short-circuits the query entirely: a section the user
        // has hidden costs nothing to not draw.
        match db.filter(|_| settings.show_recent).map(|d| {
            crate::query::recent_sessions(d, &live_ids, settings.recent_limit as usize)
        }) {
            Some(Ok(found)) => found
                .into_iter()
                .map(|r| {
                    let project = r
                        .cwd
                        .as_deref()
                        .map(project_of)
                        .unwrap_or_else(|| r.id.chars().take(8).collect());
                    let ended_ago = human_elapsed(now_ms - r.last_activity_at);
                    let tokens = if r.usage.total_tokens() > 0 {
                        human_tokens(r.usage.total_tokens())
                    } else {
                        DASH.into()
                    };
                    let ended_line = if tokens == DASH {
                        format!("ended {ended_ago} ago")
                    } else {
                        format!("{tokens} · ended {ended_ago} ago")
                    };
                    RecentRow {
                        id: r.id,
                        // Its own title, not its project's name — otherwise
                        // three finished sessions in one project render as
                        // the same word three times over.
                        name: r
                            .title
                            .clone()
                            .filter(|t| !t.trim().is_empty())
                            .unwrap_or_else(|| project.clone()),
                        project,
                        ended_ago,
                        tokens,
                        ended_line,
                    }
                })
                .collect(),
            Some(Err(_)) => {
                data_error = true;
                Vec::new()
            }
            None => Vec::new(),
        };

    if data_error {
        error.get_or_insert_with(|| "some session data unavailable".to_string());
    }

    let waiting = rows.iter().filter(|r| r.status == Status::Waiting).count();

    PopoverModel {
        stats,
        live: rows,
        recent,
        tray_title: tray_title(live, settings.menu_bar_display),
        error,
        waiting_banner: waiting_banner(waiting),
        staleness: staleness(settings, last_read_ms, now_ms),
        show_waiting: settings.show_waiting,
        show_working: settings.show_working,
        show_recent: settings.show_recent,
        row_density: settings.row_density,
        menu_bar_icon: settings.menu_bar_icon,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn, TurnUsage};
    use crate::pricing::seed_default_prices;

    /// The settings these tests ran against before `build_model` took a
    /// `&Settings`: defaults everywhere except `menu_bar_display`, which each
    /// call used to pass explicitly. Every other assertion here is therefore
    /// unchanged by the switch.
    fn count_and_waiting() -> Settings {
        Settings {
            menu_bar_display: MenuBarDisplay::CountAndWaiting,
            ..Settings::default()
        }
    }

    fn live(
        pid: i32,
        id: &str,
        name: &str,
        cwd: &str,
        status: SessionStatus,
        kind: &str,
    ) -> LiveSession {
        LiveSession {
            pid,
            session_id: id.into(),
            cwd: cwd.into(),
            name: name.into(),
            kind: kind.into(),
            status,
            started_at: 1_000,
            status_updated_at: 5_000,
            cc_version: Some("2.1.251".into()),
            socket_path: None,
        }
    }

    #[test]
    fn no_db_yields_sessions_only_with_dashes() {
        let s = live(
            1,
            "a",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Working,
            "interactive",
        );
        let m = build_model(None, &[s], 10_000, None, &count_and_waiting());
        assert!(!m.stats.has_data);
        assert_eq!(m.stats.window_tokens, "—");
        assert_eq!(m.live.len(), 1);
        assert_eq!(
            m.live[0].tokens, "—",
            "no db → no per-session usage, shown as a dash"
        );
        assert!(m.recent.is_empty());
    }

    #[test]
    fn rows_carry_project_kind_version_and_labels() {
        let s = live(
            7,
            "a",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Idle,
            "interactive",
        );
        let m = build_model(None, &[s], 10_000, None, &count_and_waiting());
        let r = &m.live[0];
        assert_eq!(r.project, "proj", "last path component of cwd");
        assert_eq!(r.kind, "interactive");
        assert_eq!(r.version, "2.1.251");
        assert_eq!(r.status, Status::Idle);
        assert_eq!(r.status_label, "idle");
        assert_eq!(r.elapsed, "5s", "now 10_000 - status_updated_at 5_000");
    }

    #[test]
    fn waiting_label_includes_the_reason_and_elapsed_since_waiting() {
        let mut s = live(7, "a", "alpha", "/x", SessionStatus::Working, "interactive");
        s.status = SessionStatus::Waiting {
            reason: Some("dialog open".into()),
            since_ms: 2_000,
        };
        let m = build_model(None, &[s], 10_000, None, &count_and_waiting());
        assert_eq!(m.live[0].status_label, "waiting · dialog open");
        assert_eq!(m.live[0].elapsed, "8s");
        assert_eq!(m.live[0].status, Status::Waiting);
    }

    #[test]
    fn absent_status_timestamp_is_a_dash() {
        let mut s = live(7, "a", "alpha", "/x", SessionStatus::Working, "interactive");
        s.status_updated_at = 0;
        let m = build_model(None, &[s], 10_000, None, &count_and_waiting());
        assert_eq!(m.live[0].elapsed, "—");
    }

    #[test]
    fn background_kind_maps_to_background_status_unless_waiting() {
        let bg = live(1, "a", "alpha", "/x", SessionStatus::Working, "bg");
        let mut bg_wait = live(2, "b", "beta", "/x", SessionStatus::Working, "bg");
        bg_wait.status = SessionStatus::Waiting {
            reason: None,
            since_ms: 1,
        };
        let m = build_model(None, &[bg, bg_wait], 10_000, None, &count_and_waiting());
        let by_id = |id: &str| m.live.iter().find(|r| r.id == id).unwrap();
        assert_eq!(by_id("a").status, Status::Background);
        assert_eq!(
            by_id("b").status,
            Status::Waiting,
            "a blocked bg session is still blocked"
        );
    }

    #[test]
    fn tray_title_counts_and_flags_waiting() {
        let w = {
            let mut s = live(1, "a", "a", "/x", SessionStatus::Working, "interactive");
            s.status = SessionStatus::Waiting {
                reason: None,
                since_ms: 1,
            };
            s
        };
        let b = live(2, "b", "b", "/x", SessionStatus::Working, "interactive");
        assert_eq!(tray_title(&[], MenuBarDisplay::CountAndWaiting), "");
        assert_eq!(
            tray_title(std::slice::from_ref(&b), MenuBarDisplay::CountAndWaiting),
            "1"
        );
        assert_eq!(tray_title(&[w, b], MenuBarDisplay::CountAndWaiting), "1 ⏳");
    }

    #[test]
    fn tray_title_icon_only_is_always_blank() {
        let w = {
            let mut s = live(1, "a", "a", "/x", SessionStatus::Working, "interactive");
            s.status = SessionStatus::Waiting {
                reason: None,
                since_ms: 1,
            };
            s
        };
        let b = live(2, "b", "b", "/x", SessionStatus::Working, "interactive");
        assert_eq!(tray_title(&[], MenuBarDisplay::Icon), "");
        assert_eq!(
            tray_title(std::slice::from_ref(&b), MenuBarDisplay::Icon),
            "",
            "Icon-only must never show a count"
        );
        assert_eq!(
            tray_title(&[w, b], MenuBarDisplay::Icon),
            "",
            "Icon-only must never show the waiting hourglass either"
        );
    }

    #[test]
    fn tray_title_count_never_shows_the_hourglass() {
        let w = {
            let mut s = live(1, "a", "a", "/x", SessionStatus::Working, "interactive");
            s.status = SessionStatus::Waiting {
                reason: None,
                since_ms: 1,
            };
            s
        };
        let b = live(2, "b", "b", "/x", SessionStatus::Working, "interactive");
        assert_eq!(tray_title(&[], MenuBarDisplay::Count), "");
        assert_eq!(
            tray_title(std::slice::from_ref(&b), MenuBarDisplay::Count),
            "1"
        );
        assert_eq!(
            tray_title(&[w, b], MenuBarDisplay::Count),
            "2",
            "Count must report the plain live count, not the waiting count, \
             and never the hourglass"
        );
    }

    #[test]
    fn waiting_banner_is_pluralised_by_count() {
        let cases: &[(usize, Option<&str>)] = &[
            (0, None),
            (1, Some("1 session is waiting on you")),
            (2, Some("2 sessions are waiting on you")),
            (5, Some("5 sessions are waiting on you")),
        ];
        for &(count, expected) in cases {
            assert_eq!(
                waiting_banner(count),
                expected.map(str::to_string),
                "count={count}"
            );
        }
    }

    #[test]
    fn build_model_surfaces_the_waiting_banner() {
        let mut w = live(1, "a", "a", "/x", SessionStatus::Working, "interactive");
        w.status = SessionStatus::Waiting {
            reason: None,
            since_ms: 1,
        };
        let b = live(2, "b", "b", "/x", SessionStatus::Working, "interactive");

        assert_eq!(
            build_model(None, &[], 10_000, None, &count_and_waiting()).waiting_banner,
            None
        );
        assert_eq!(
            build_model(
                None,
                std::slice::from_ref(&b),
                10_000,
                None,
                &count_and_waiting()
            )
            .waiting_banner,
            None
        );
        assert_eq!(
            build_model(None, &[w], 10_000, None, &count_and_waiting()).waiting_banner,
            Some("1 session is waiting on you".to_string())
        );
    }

    #[test]
    fn stats_and_per_session_usage_come_from_the_index() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-proj", "/Users/a/proj", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            id: "a".into(),
            project_id: pid,
            file_path: "/tmp/a.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(1_000),
            last_activity_at: Some(9_000),
            cwd: Some("/Users/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 1,
        })
        .unwrap();
        db.insert_turns(
            "a",
            &[Turn {
                ts: 9_000,
                model: "claude-fable-5".into(),
                usage: TurnUsage {
                    input: 2_000_000,
                    ..Default::default()
                },
            }],
        )
        .unwrap();

        let s = live(
            7,
            "a",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Working,
            "interactive",
        );
        let m = build_model(Some(&db), &[s], 10_000, None, &count_and_waiting());
        assert!(m.stats.has_data);
        assert!(m.stats.estimated);
        assert_eq!(m.stats.window_tokens, "2.0M");
        assert_eq!(m.stats.day_cost, "$30.00");
        assert_eq!(m.live[0].tokens, "2.0M");
        assert_eq!(m.live[0].cost, "$30.00");
        assert_eq!(m.error, None, "the healthy path must not report an error");
    }

    #[test]
    fn recent_excludes_live_and_names_by_project() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-proj", "/Users/a/proj", false)
            .unwrap();
        for (id, last) in [("live", 9_000), ("gone", 8_000)] {
            db.upsert_session(&SessionRecord {
                id: id.into(),
                project_id: pid,
                file_path: format!("/tmp/{id}.jsonl"),
                file_size: 0,
                indexed_offset: 0,
                started_at: Some(1),
                last_activity_at: Some(last),
                cwd: Some("/Users/a/proj".into()),
                git_branch: None,
                cc_version: None,
                title: None,
                message_count: 1,
            })
            .unwrap();
        }
        let s = live(
            7,
            "live",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Working,
            "interactive",
        );
        let m = build_model(Some(&db), &[s], 10_000, None, &count_and_waiting());
        assert_eq!(m.recent.len(), 1);
        assert_eq!(m.recent[0].id, "gone");
        assert_eq!(m.recent[0].project, "proj");
        assert_eq!(m.recent[0].ended_ago, "2s");
        assert_eq!(m.error, None, "the healthy path must not report an error");
    }

    /// `detail_line` is the composed "project · kind · vVERSION · TOKENS ·
    /// COST" a shell renders verbatim. Table-tests every combination of
    /// present/absent version and usage, including an empty (not just
    /// missing) version string.
    #[test]
    fn detail_line_omits_absent_parts_and_includes_present_ones() {
        // (cc_version, has_usage_in_db, expected detail_line)
        let cases: &[(Option<&str>, bool, &str)] = &[
            (None, false, "proj · interactive"),
            (Some(""), false, "proj · interactive"),
            (Some("2.1.251"), false, "proj · interactive · v2.1.251"),
            (None, true, "proj · interactive · 2.0M · $30.00"),
            (
                Some("2.1.251"),
                true,
                "proj · interactive · v2.1.251 · 2.0M · $30.00",
            ),
        ];

        for &(version, has_usage, expected) in cases {
            let db = has_usage.then(|| {
                let db = open_in_memory().unwrap();
                seed_default_prices(&db).unwrap();
                let pid = db
                    .upsert_project("-Users-a-proj", "/Users/a/proj", false)
                    .unwrap();
                db.upsert_session(&SessionRecord {
                    id: "a".into(),
                    project_id: pid,
                    file_path: "/tmp/a.jsonl".into(),
                    file_size: 0,
                    indexed_offset: 0,
                    started_at: Some(1_000),
                    last_activity_at: Some(9_000),
                    cwd: Some("/Users/a/proj".into()),
                    git_branch: None,
                    cc_version: None,
                    title: None,
                    message_count: 1,
                })
                .unwrap();
                db.insert_turns(
                    "a",
                    &[Turn {
                        ts: 9_000,
                        model: "claude-fable-5".into(),
                        usage: TurnUsage {
                            input: 2_000_000,
                            ..Default::default()
                        },
                    }],
                )
                .unwrap();
                db
            });

            let mut s = live(
                7,
                "a",
                "alpha",
                "/Users/a/proj",
                SessionStatus::Working,
                "interactive",
            );
            s.cc_version = version.map(str::to_string);
            let m = build_model(db.as_ref(), &[s], 10_000, None, &count_and_waiting());
            assert_eq!(
                m.live[0].detail_line, expected,
                "version={version:?} has_usage={has_usage}"
            );
        }
    }

    /// `ended_line` is the composed "TOKENS · ended AGO ago" (or, with no
    /// usage, just "ended AGO ago") a shell renders verbatim for a recent row.
    #[test]
    fn ended_line_omits_tokens_when_absent_and_includes_them_when_present() {
        for (session_id, add_turns, expected) in [
            ("gone", false, "ended 2s ago"),
            ("gone-with-usage", true, "2.0M · ended 2s ago"),
        ] {
            let db = open_in_memory().unwrap();
            seed_default_prices(&db).unwrap();
            let pid = db
                .upsert_project("-Users-a-proj", "/Users/a/proj", false)
                .unwrap();
            db.upsert_session(&SessionRecord {
                id: session_id.into(),
                project_id: pid,
                file_path: format!("/tmp/{session_id}.jsonl"),
                file_size: 0,
                indexed_offset: 0,
                started_at: Some(1),
                last_activity_at: Some(8_000),
                cwd: Some("/Users/a/proj".into()),
                git_branch: None,
                cc_version: None,
                title: None,
                message_count: 1,
            })
            .unwrap();
            if add_turns {
                db.insert_turns(
                    session_id,
                    &[Turn {
                        ts: 8_000,
                        model: "claude-fable-5".into(),
                        usage: TurnUsage {
                            input: 2_000_000,
                            ..Default::default()
                        },
                    }],
                )
                .unwrap();
            }
            let m = build_model(Some(&db), &[], 10_000, None, &count_and_waiting());
            assert_eq!(m.recent.len(), 1);
            assert_eq!(m.recent[0].ended_line, expected, "session_id={session_id}");
        }
    }

    /// How many ended sessions the popover's Recent section lists is the
    /// user's `recent_limit`, not a constant — and the default still lists
    /// three, so unfreezing the constant changed nothing a user sees.
    #[test]
    fn the_recent_section_honours_its_setting() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-proj", "/Users/a/proj", false)
            .unwrap();
        for i in 0..10i64 {
            db.upsert_session(&SessionRecord {
                id: format!("ended-{i}"),
                project_id: pid,
                file_path: format!("/tmp/ended-{i}.jsonl"),
                file_size: 0,
                indexed_offset: 0,
                started_at: Some(1_000 + i),
                last_activity_at: Some(1_000 + i),
                cwd: Some("/Users/a/proj".into()),
                git_branch: None,
                cc_version: None,
                title: None,
                message_count: 1,
            })
            .unwrap();
        }

        let two = Settings {
            recent_limit: 2,
            ..Settings::default()
        };
        assert_eq!(
            build_model(Some(&db), &[], 10_000, None, &two).recent.len(),
            2
        );
        assert_eq!(
            build_model(Some(&db), &[], 10_000, None, &Settings::default())
                .recent
                .len(),
            3,
            "the default still lists three, as RECENT_LIMIT did"
        );
    }

    /// Break only `recent_sessions` (it selects `sessions.cwd`, renamed away
    /// here) while leaving `turns` intact, so `stats_from` and per-row
    /// `session_usage` (both read only `turns`) stay healthy. Dropping the
    /// `sessions` table outright would fail on its own — `turns.session_id`
    /// has a live foreign key into it — so a column rename is used instead
    /// to isolate the failure to the one query. Proves a failure isolated to
    /// the recent-list query is surfaced, not swallowed by `.ok()`.
    #[test]
    fn recent_sessions_failure_is_surfaced_without_breaking_stats_or_rows() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-proj", "/Users/a/proj", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            id: "a".into(),
            project_id: pid,
            file_path: "/tmp/a.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(1_000),
            last_activity_at: Some(9_000),
            cwd: Some("/Users/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 1,
        })
        .unwrap();
        db.insert_turns(
            "a",
            &[Turn {
                ts: 9_000,
                model: "claude-fable-5".into(),
                usage: TurnUsage {
                    input: 2_000_000,
                    ..Default::default()
                },
            }],
        )
        .unwrap();
        db.conn()
            .execute("ALTER TABLE sessions RENAME COLUMN cwd TO cwd_renamed", [])
            .unwrap();

        let s = live(
            7,
            "a",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Working,
            "interactive",
        );
        let m = build_model(Some(&db), &[s], 10_000, None, &count_and_waiting());
        assert_eq!(
            m.error.as_deref(),
            Some("some session data unavailable"),
            "a recent_sessions failure must not vanish silently"
        );
        assert!(
            m.stats.has_data,
            "stats reads only `turns`, which is intact"
        );
        assert_eq!(m.stats.window_tokens, "2.0M");
        assert_eq!(
            m.live[0].tokens, "2.0M",
            "per-row usage reads only `turns`, which is intact"
        );
        assert!(
            m.recent.is_empty(),
            "degrades to an empty list, not a panic"
        );
    }

    /// Break `turns` itself, which both `stats_from` and the row/recent
    /// queries depend on. `stats_from` runs first and sets the specific
    /// "index unavailable" message; the row- and recent-level failures that
    /// follow must not clobber it with the generic message.
    #[test]
    fn a_more_specific_stats_error_is_not_clobbered_by_row_level_failures() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-proj", "/Users/a/proj", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            id: "a".into(),
            project_id: pid,
            file_path: "/tmp/a.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(1_000),
            last_activity_at: Some(9_000),
            cwd: Some("/Users/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 1,
        })
        .unwrap();
        db.conn().execute("DROP TABLE turns", []).unwrap();

        let s = live(
            7,
            "a",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Working,
            "interactive",
        );
        let m = build_model(Some(&db), &[s], 10_000, None, &count_and_waiting());
        let msg = m.error.expect("a broken index must report an error");
        assert!(
            msg.starts_with("index unavailable"),
            "the specific stats_from message must win, got: {msg}"
        );
        assert!(!m.stats.has_data);
        assert_eq!(m.stats.window_tokens, "—");
        assert_eq!(m.live[0].tokens, "—");
        assert!(m.recent.is_empty());
    }

    const MIN: i64 = 60_000;

    /// One priced session with real usage, so every cost-bearing surface of
    /// the popover model has something on it to hide.
    fn priced_db() -> Db {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-proj", "/Users/a/proj", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            id: "a".into(),
            project_id: pid,
            file_path: "/tmp/a.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(1_000),
            last_activity_at: Some(9_000),
            cwd: Some("/Users/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 1,
        })
        .unwrap();
        db.insert_turns(
            "a",
            &[Turn {
                ts: 9_000,
                model: "claude-fable-5".into(),
                usage: TurnUsage {
                    input: 2_000_000,
                    ..Default::default()
                },
            }],
        )
        .unwrap();
        db
    }

    #[test]
    fn data_is_not_stale_before_the_threshold() {
        let s = Settings::default(); // dim_when_stale, 5 minutes
        let now = 100 * MIN;
        let m = build_model(None, &[], now, Some(now - 2 * MIN), &s);
        assert!(m.staleness.is_none());
    }

    #[test]
    fn stale_data_says_how_old_it_is_in_finished_words() {
        let s = Settings::default();
        let now = 100 * MIN;
        let m = build_model(None, &[], now, Some(now - 9 * MIN), &s);
        let stale = m
            .staleness
            .expect("nine minutes is past a five minute threshold");
        assert_eq!(stale.label, "Last updated 9m ago");
    }

    #[test]
    fn the_threshold_itself_is_not_yet_stale() {
        let s = Settings::default();
        let now = 100 * MIN;
        assert!(
            build_model(None, &[], now, Some(now - 5 * MIN), &s)
                .staleness
                .is_none(),
            "stale once the gap *exceeds* the threshold, not on reaching it"
        );
    }

    #[test]
    fn staleness_is_never_reported_when_dimming_is_off() {
        let s = Settings {
            dim_when_stale: false,
            ..Settings::default()
        };
        let now = 1_000 * MIN;
        let m = build_model(None, &[], now, Some(now - 99 * MIN), &s);
        assert!(
            m.staleness.is_none(),
            "the setting is off; there is nothing to draw"
        );
    }

    #[test]
    fn a_never_read_index_is_not_reported_as_stale() {
        let s = Settings::default();
        assert!(
            build_model(None, &[], 100 * MIN, None, &s)
                .staleness
                .is_none(),
            "nothing has been read, so no reading has aged; `error` tells that story"
        );
    }

    #[test]
    fn section_visibility_and_density_are_decided_here_not_in_the_shell() {
        let s = Settings {
            show_waiting: false,
            show_working: true,
            show_recent: false,
            row_density: RowDensity::Compact,
            ..Settings::default()
        };
        let m = build_model(Some(&priced_db()), &[], 10_000, None, &s);
        assert!(!m.show_waiting);
        assert!(m.show_working);
        assert!(!m.show_recent);
        assert_eq!(m.row_density, RowDensity::Compact);
        assert!(
            m.recent.is_empty(),
            "a hidden section is not queried, let alone drawn"
        );
    }

    #[test]
    fn the_chosen_menu_bar_icon_reaches_the_model() {
        let s = Settings {
            menu_bar_icon: MenuBarIcon::Binoculars,
            ..Settings::default()
        };
        assert_eq!(
            build_model(None, &[], 10_000, None, &s).menu_bar_icon,
            MenuBarIcon::Binoculars,
            "the shell maps the variant to a glyph; it must not read the setting itself"
        );
        assert_eq!(
            PopoverModel::empty().menu_bar_icon,
            Settings::default().menu_bar_icon,
            "the pre-first-tick model draws the shipped default, not a second one"
        );
    }

    #[test]
    fn the_row_folder_appears_only_when_asked_for() {
        let sess = live(
            7,
            "a",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Working,
            "interactive",
        );
        let off = build_model(
            None,
            std::slice::from_ref(&sess),
            10_000,
            None,
            &count_and_waiting(),
        );
        assert_eq!(off.live[0].folder, "");
        let s = Settings {
            show_row_folder: true,
            ..count_and_waiting()
        };
        let on = build_model(None, &[sess], 10_000, None, &s);
        assert_eq!(on.live[0].folder, "/Users/a/proj");
    }

    #[test]
    fn hiding_row_usage_takes_the_tokens_and_the_cost_off_the_row() {
        let sess = live(
            7,
            "a",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Working,
            "interactive",
        );
        let s = Settings {
            show_row_usage: false,
            ..count_and_waiting()
        };
        let m = build_model(Some(&priced_db()), &[sess], 10_000, None, &s);
        assert_eq!(m.live[0].tokens, "");
        assert_eq!(m.live[0].cost, "");
        assert_eq!(
            m.live[0].detail_line, "proj · interactive · v2.1.251",
            "and the composed line drops the segment too"
        );
    }

    #[test]
    fn hiding_cost_hides_it_everywhere_the_popover_would_show_it() {
        let sess = live(
            7,
            "a",
            "alpha",
            "/Users/a/proj",
            SessionStatus::Working,
            "interactive",
        );
        let s = Settings {
            show_cost: false,
            ..count_and_waiting()
        };
        let m = build_model(Some(&priced_db()), &[sess], 10_000, None, &s);
        assert!(!m.stats.day_cost.contains('$'), "{:?}", m.stats.day_cost);
        assert!(m.live.iter().all(|r| !r.cost.contains('$')));
        assert!(m.live.iter().all(|r| !r.detail_line.contains('$')));
        assert!(m.recent.iter().all(|r| !r.ended_line.contains('$')));
        assert_eq!(
            m.live[0].tokens, "2.0M",
            "tokens are counted, not inferred; only the dollars go"
        );
        assert_eq!(
            m.live[0].detail_line,
            "proj · interactive · v2.1.251 · 2.0M"
        );
    }
}

#[cfg(test)]
mod title_tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::SessionRecord;

    /// A session's own title is the point of having indexed one. The popover
    /// showed Claude Code's `<project>-<id>` slug for live rows and, worse,
    /// the *project* name for recent ones — so three sessions in one project
    /// read as the same word three times.
    fn seed(db: &crate::db::Db, id: &str, cwd: &str, title: Option<&str>, last: i64) {
        let pid = db.upsert_project("-work-api", "/work/api", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: id.into(),
            project_id: pid,
            file_path: format!("/tmp/{id}.jsonl"),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(1),
            last_activity_at: Some(last),
            cwd: Some(cwd.into()),
            git_branch: None,
            cc_version: None,
            title: title.map(Into::into),
            message_count: 1,
        })
        .unwrap();
    }

    fn running(id: &str, name: &str, cwd: &str) -> LiveSession {
        LiveSession {
            pid: 1,
            session_id: id.into(),
            cwd: cwd.into(),
            name: name.into(),
            kind: "interactive".into(),
            status: SessionStatus::Working,
            started_at: 1,
            status_updated_at: 5,
            cc_version: None,
            socket_path: None,
        }
    }

    #[test]
    fn a_live_row_prefers_the_indexed_title_over_the_slug() {
        let db = open_in_memory().unwrap();
        seed(
            &db,
            "s1",
            "/work/api",
            Some("Pricing module security review"),
            9,
        );
        let m = build_model(
            Some(&db),
            &[running("s1", "api-s1", "/work/api")],
            10,
            None,
            &Settings::default(),
        );
        assert_eq!(m.live[0].name, "Pricing module security review");
    }

    #[test]
    fn a_live_row_falls_back_to_the_record_name_when_untitled() {
        let db = open_in_memory().unwrap();
        seed(&db, "s1", "/work/api", None, 9);
        let m = build_model(
            Some(&db),
            &[running("s1", "api-s1", "/work/api")],
            10,
            None,
            &Settings::default(),
        );
        assert_eq!(
            m.live[0].name, "api-s1",
            "an untitled session is not a reason to show nothing"
        );
    }

    #[test]
    fn recent_rows_show_the_session_not_its_project() {
        let db = open_in_memory().unwrap();
        seed(
            &db,
            "a",
            "/work/api",
            Some("Pricing module security review"),
            10,
        );
        seed(&db, "b", "/work/api", Some("ci.yml security hardening"), 20);
        let m = build_model(Some(&db), &[], 100, None, &Settings::default());
        let names: Vec<&str> = m.recent.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.contains(&"Pricing module security review"),
            "got {names:?}"
        );
        assert!(
            names.contains(&"ci.yml security hardening"),
            "got {names:?}"
        );
        assert_ne!(
            names[0], names[1],
            "two sessions in one project must not read alike"
        );
    }

    #[test]
    fn an_untitled_recent_row_still_names_its_project() {
        let db = open_in_memory().unwrap();
        seed(&db, "a", "/work/api", None, 10);
        let m = build_model(Some(&db), &[], 100, None, &Settings::default());
        assert_eq!(m.recent[0].name, "api");
    }
}
