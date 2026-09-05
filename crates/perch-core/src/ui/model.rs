//! The view-model every shell renders. All strings are final here.

use crate::db::Db;
use crate::live::{LiveSession, SessionStatus};
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
    pub tokens: String,
    pub cost: String,
    /// The composed "project · kind · vVERSION · TOKENS · COST" line, omitting
    /// whichever parts are absent — a shell renders this verbatim rather than
    /// assembling it (and rather than comparing `tokens`/`version` against a
    /// sentinel to decide what to omit; that comparison is Rust's to make).
    /// `project`, `kind`, `version`, `tokens`, and `cost` stay on the row too,
    /// for a shell that wants the parts separately.
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
}

const DASH: &str = "—";
const RECENT_LIMIT: usize = 3;
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

/// The waiting-sessions banner sentence, or `None` when nothing is waiting.
/// Pluralisation lives here so no shell has to re-derive it from a count.
fn waiting_banner(waiting: usize) -> Option<String> {
    match waiting {
        0 => None,
        1 => Some("1 session is waiting on you".to_string()),
        n => Some(format!("{n} sessions are waiting on you")),
    }
}

/// Menu-bar text: blocked count with an hourglass, else live count, else nothing.
pub fn tray_title(live: &[LiveSession]) -> String {
    let waiting = live
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Waiting { .. }))
        .count();
    if waiting > 0 {
        format!("{waiting} ⏳")
    } else if live.is_empty() {
        String::new()
    } else {
        live.len().to_string()
    }
}

fn dashed_stats() -> Stats {
    Stats {
        window_tokens: DASH.into(),
        week_tokens: DASH.into(),
        day_tokens: DASH.into(),
        day_cost: DASH.into(),
        estimated: true,
        has_data: false,
    }
}

/// `db.rs` uses `anyhow::Result` throughout (no local `Result` alias), so this
/// does too — the caller in `build_model` collapses any error to a dash.
fn stats_from(db: &Db, now_ms: i64) -> anyhow::Result<Stats> {
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
        day_cost: human_cost(d_cost),
        estimated: true,
        has_data: true,
    })
}

/// Build the whole view-model. `db: None` (or a failing db) degrades to sessions-only:
/// the sessions list needs no index, and an honest dash beats a fabricated zero.
pub fn build_model(db: Option<&Db>, live: &[LiveSession], now_ms: i64) -> PopoverModel {
    let mut error = None;

    let stats = match db.map(|d| stats_from(d, now_ms)) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            error = Some(format!("index unavailable: {e}"));
            dashed_stats()
        }
        None => dashed_stats(),
    };

    // A row- or recent-list-level query failure still degrades honestly (a dash,
    // an empty list — never a fabricated zero), but must not vanish silently:
    // flag it here and fold it into `error` below, without clobbering a more
    // specific message `stats_from` may already have set.
    let mut data_error = false;

    let rows: Vec<SessionRow> = live
        .iter()
        .map(|s| {
            let (status, status_label, since) = status_of(s);
            let (tokens, cost) = match db.map(|d| crate::query::session_usage(d, &s.session_id)) {
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
            let project = project_of(&s.cwd);
            let kind = s.kind.clone();
            let version = s.cc_version.clone().unwrap_or_default();
            let detail_line = [
                Some(project.clone()),
                Some(kind.clone()),
                (!version.is_empty()).then(|| format!("v{version}")),
                (tokens != DASH).then(|| format!("{tokens} · {cost}")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            SessionRow {
                id: s.session_id.clone(),
                pid: s.pid,
                name: if s.name.is_empty() {
                    s.session_id.chars().take(8).collect()
                } else {
                    s.name.clone()
                },
                project,
                kind,
                version,
                status,
                status_label,
                elapsed: elapsed_or_dash(now_ms, since),
                tokens,
                cost,
                detail_line,
            }
        })
        .collect();

    let live_ids: Vec<String> = live.iter().map(|s| s.session_id.clone()).collect();
    let recent: Vec<RecentRow> =
        match db.map(|d| crate::query::recent_sessions(d, &live_ids, RECENT_LIMIT)) {
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
                        name: project.clone(),
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
        tray_title: tray_title(live),
        error,
        waiting_banner: waiting_banner(waiting),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn, TurnUsage};
    use crate::pricing::seed_default_prices;

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
        let m = build_model(None, &[s], 10_000);
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
        let m = build_model(None, &[s], 10_000);
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
        let m = build_model(None, &[s], 10_000);
        assert_eq!(m.live[0].status_label, "waiting · dialog open");
        assert_eq!(m.live[0].elapsed, "8s");
        assert_eq!(m.live[0].status, Status::Waiting);
    }

    #[test]
    fn absent_status_timestamp_is_a_dash() {
        let mut s = live(7, "a", "alpha", "/x", SessionStatus::Working, "interactive");
        s.status_updated_at = 0;
        let m = build_model(None, &[s], 10_000);
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
        let m = build_model(None, &[bg, bg_wait], 10_000);
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
        assert_eq!(tray_title(&[]), "");
        assert_eq!(tray_title(std::slice::from_ref(&b)), "1");
        assert_eq!(tray_title(&[w, b]), "1 ⏳");
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

        assert_eq!(build_model(None, &[], 10_000).waiting_banner, None);
        assert_eq!(
            build_model(None, std::slice::from_ref(&b), 10_000).waiting_banner,
            None
        );
        assert_eq!(
            build_model(None, &[w], 10_000).waiting_banner,
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
        let m = build_model(Some(&db), &[s], 10_000);
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
        let m = build_model(Some(&db), &[s], 10_000);
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
            let m = build_model(db.as_ref(), &[s], 10_000);
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
            let m = build_model(Some(&db), &[], 10_000);
            assert_eq!(m.recent.len(), 1);
            assert_eq!(m.recent[0].ended_line, expected, "session_id={session_id}");
        }
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
        let m = build_model(Some(&db), &[s], 10_000);
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
        let m = build_model(Some(&db), &[s], 10_000);
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
}
