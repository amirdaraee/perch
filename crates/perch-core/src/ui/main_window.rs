//! The main window's view-model: every project the index knows, and one
//! project's full history. Same rule as the popover — every string is final here.

use crate::db::{Db, NotifyOverride};
use crate::live::{LiveSession, SessionStatus};
use crate::query;
use crate::readme;
use crate::settings::Settings;
use crate::ui::format::{elapsed_or_dash, human_cost, human_elapsed, human_tokens, plural};
use crate::ui::model::PopoverModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ProjectGroup {
    Pinned,
    Active,
    Recent,
    Archived,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectRow {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub group: ProjectGroup,
    pub session_count: String,
    pub tokens: String,
    pub cost: String,
    pub last_active: String,
    pub live_session_count: u32,
    /// "N sessions · 2.0M · $30.00 · 1h" — composed here so no shell rebuilds it.
    pub subtitle: String,
    /// The overview card's opening paragraph from the project's own README or
    /// CLAUDE.md; `None` when neither says anything or the folder is gone.
    pub description: Option<String>,
    /// Tokens per day over `settings.chart_days`, oldest first — the same
    /// window and labels as the project page's sparkline.
    pub sparkline: Vec<SparkPoint>,
    /// "2 running", or `None` when nothing runs in this project.
    pub live_label: Option<String>,
    /// "1 waiting", or `None` when no session here is waiting on the user.
    pub waiting_label: Option<String>,
    /// "12 sessions · 2.0M · $30.00" — the subtitle without its recency.
    pub stats_line: String,
    /// "main · 3h" — the latest branch, when one was recorded, then recency.
    pub activity_line: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SparkPoint {
    pub day_index: i32,
    pub tokens: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SessionHistoryRow {
    pub id: String,
    pub name: String,
    pub started: String,
    pub duration: String,
    pub tokens: String,
    pub cost: String,
    pub branch: Option<String>,
    pub is_live: bool,
    pub detail_line: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectDetail {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub note: String,
    pub tokens: String,
    pub cost: String,
    pub session_count: String,
    pub sparkline: Vec<SparkPoint>,
    pub sessions: Vec<SessionHistoryRow>,
    pub pinned: bool,
    pub archived: bool,
    pub path_exists: bool,
    /// This project's own notification override — not the global setting.
    pub notify: NotifyOverride,
    /// What `NotifyOverride::Default` currently means, spelled out: the
    /// global "waiting on you" threshold as a finished, user-facing sentence
    /// fragment, always present regardless of `notify`'s own value. The
    /// Default/Custom/Off control (spec: per-project override, in the
    /// project detail pane) shows this beside the control when Default is
    /// selected, so the user is never choosing blind — composed here, not
    /// assembled from a raw number in a shell, for the same reason every
    /// other string on this struct is finished in Rust.
    pub notify_default_label: String,
    /// The same global threshold `notify_default_label` spells out, as the
    /// number itself: the value a shell's "Custom" minute control starts at
    /// before the user has chosen one. It exists so that seeding is a read
    /// of this field rather than a literal in the shell (which drifts the
    /// moment the default moves) or a parse of the label above (which is
    /// prose, and not a data format). The two are always composed from the
    /// same value and can never disagree.
    pub notify_default_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MainWindowModel {
    pub now: PopoverModel,
    pub projects: Vec<ProjectRow>,
    pub error: Option<String>,
}

fn dir_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// What `NotifyOverride::Default` currently means: the global
/// `waiting_after_minutes` setting, spelled out as a finished sentence
/// fragment rather than a bare number a shell would have to pluralize and
/// contextualize itself.
fn default_notify_label(waiting_after_minutes: u32) -> String {
    format!(
        "Default — waits {}",
        plural(i64::from(waiting_after_minutes), "minute", "minutes")
    )
}

/// The `NotifyOverride::Custom` counterpart to `default_notify_label`, for the
/// value a shell's stepper is *currently* showing rather than one already
/// stored. It exists so that number is pluralized here and not in the shell:
/// the sibling above is only reached for a persisted setting, which left the
/// live stepper as the one place a UI still had to compose a sentence itself.
pub fn custom_notify_label(minutes: u32) -> String {
    format!("After {}", plural(i64::from(minutes), "minute", "minutes"))
}

/// `elapsed_or_dash` already treats a non-positive timestamp as absent; this
/// just extends that to the `Option` these queries actually return, rather
/// than re-deciding what "absent" means a second time.
fn since(now_ms: i64, ts: Option<i64>) -> String {
    ts.map_or_else(|| "—".to_string(), |t| elapsed_or_dash(now_ms, t))
}

/// Every project the index knows, grouped for the sidebar. `db: None` (or a
/// failing read) yields an empty list with the reason attached — never a silently
/// empty window.
///
/// `settings.active_within_days` is the Active/Recent boundary — the user's
/// window, not a frozen seven days.
pub fn build_main_window(
    db: Option<&Db>,
    now: PopoverModel,
    live: &[LiveSession],
    now_ms: i64,
    settings: &Settings,
) -> MainWindowModel {
    let Some(db) = db else {
        return MainWindowModel {
            now,
            projects: Vec::new(),
            error: Some("index unavailable".into()),
        };
    };
    let active_window_ms = i64::from(settings.active_within_days) * query::DAY_MS;
    let chart_days = settings.chart_days as usize;
    // All three are read up front, so a card's sparkline and branch cost one
    // query each for the whole grid rather than one per project. A failure in
    // any of them fails the list: zero bars would be a fabricated "quiet".
    let loaded = query::project_summaries(db).and_then(|s| {
        Ok((
            s,
            query::daily_tokens_by_project(db, chart_days, now_ms)?,
            query::latest_branches(db)?,
        ))
    });
    let (summaries, daily, branches) = match loaded {
        Ok(s) => s,
        Err(e) => {
            return MainWindowModel {
                now,
                projects: Vec::new(),
                error: Some(format!("index unavailable: {e}")),
            }
        }
    };

    // Carry `last_activity_at` alongside each row only for sorting — it does
    // not belong on `ProjectRow` itself (the brief's fields are exact, and a
    // shell gets `last_active` already formatted), but the sidebar's order
    // still has to be decided here, not by whichever shell renders it.
    let mut projects: Vec<(ProjectRow, Option<i64>)> = summaries
        .into_iter()
        .filter_map(|s| {
            let meta = db.project_meta(s.id).ok();
            let archived = meta.as_ref().is_some_and(|m| m.archived);
            // "Show archived projects" is a grouping decision, so it is made
            // here rather than by whichever shell draws the sidebar. Hidden
            // means gone, not demoted: an archived project must never
            // reappear under Pinned or Recent because the group that owned it
            // was suppressed.
            if archived && !settings.show_archived {
                return None;
            }
            let pinned = meta.as_ref().is_some_and(|m| m.pinned);
            let group = if archived {
                ProjectGroup::Archived
            } else if pinned {
                ProjectGroup::Pinned
            } else if s
                .last_activity_at
                .is_some_and(|t| now_ms - t <= active_window_ms)
            {
                ProjectGroup::Active
            } else {
                ProjectGroup::Recent
            };
            let name = meta
                .as_ref()
                .and_then(|m| m.display_name.clone())
                .unwrap_or_else(|| dir_name(&s.real_path));
            let session_count = plural(s.sessions, "session", "sessions");
            let tokens = human_tokens(s.usage.total_tokens());
            // Hidden means absent, not "$0.00" — same rule `ui::model` and
            // `ui::usage` already follow: a blank string here says "the user
            // hid this", not "this project cost nothing".
            let cost = if settings.show_cost {
                human_cost(s.cost_usd)
            } else {
                String::new()
            };
            let last_active = since(now_ms, s.last_activity_at);
            let here: Vec<&LiveSession> = live.iter().filter(|l| l.cwd == s.real_path).collect();
            let live_session_count = here.len() as u32;
            let waiting = here
                .iter()
                .filter(|l| matches!(l.status, SessionStatus::Waiting { .. }))
                .count();
            let stats_line = [
                Some(session_count.clone()),
                Some(tokens.clone()),
                (!cost.is_empty()).then(|| cost.clone()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let activity_line = [branches.get(&s.id).cloned(), Some(last_active.clone())]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let tokens_per_day = daily
                .get(&s.id)
                .cloned()
                .unwrap_or_else(|| vec![0; chart_days]);
            // Composed as optional fragments, not a fixed-arity `format!`, so
            // a hidden cost drops cleanly instead of leaving a stray " · ".
            let subtitle = [
                Some(session_count.clone()),
                Some(tokens.clone()),
                (!cost.is_empty()).then(|| cost.clone()),
                Some(last_active.clone()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let row = ProjectRow {
                id: s.id,
                name,
                path: s.real_path.clone(),
                group,
                subtitle,
                session_count,
                tokens,
                cost,
                last_active,
                live_session_count,
                description: readme::project_description(std::path::Path::new(&s.real_path)),
                sparkline: spark_points(&tokens_per_day),
                live_label: (live_session_count > 0)
                    .then(|| format!("{live_session_count} running")),
                waiting_label: (waiting > 0).then(|| format!("{waiting} waiting")),
                stats_line,
                activity_line,
            };
            Some((row, s.last_activity_at))
        })
        .collect();

    // The sidebar's order: group first (in the priority declared on
    // `ProjectGroup` — Pinned, Active, Recent, Archived last), then within a
    // group by recency (nulls last), then by name — never left for a shell to
    // reconstruct from an unordered list.
    projects.sort_by(|(a, a_ts), (b, b_ts)| {
        group_rank(a.group)
            .cmp(&group_rank(b.group))
            .then_with(|| recency_key(*a_ts).cmp(&recency_key(*b_ts)))
            .then_with(|| a.name.cmp(&b.name))
    });
    MainWindowModel {
        now,
        projects: projects.into_iter().map(|(row, _)| row).collect(),
        error: None,
    }
}

/// One bar per day, oldest first, labelled with the same scheme as
/// `ui::usage`'s daily chart over the identical window — "Today", "1d" … — not
/// `human_elapsed`, which caps at hours and would read "313h ago" for day 0.
/// Shared by the overview card and the project page so the two never disagree.
fn spark_points(tokens_per_day: &[u64]) -> Vec<SparkPoint> {
    let n = tokens_per_day.len();
    tokens_per_day
        .iter()
        .enumerate()
        .map(|(i, &tokens)| {
            let days_ago = n - 1 - i;
            SparkPoint {
                day_index: i as i32,
                tokens,
                label: if days_ago == 0 {
                    "Today".to_string()
                } else {
                    format!("{days_ago}d")
                },
            }
        })
        .collect()
}

fn group_rank(g: ProjectGroup) -> u8 {
    match g {
        ProjectGroup::Pinned => 0,
        ProjectGroup::Active => 1,
        ProjectGroup::Recent => 2,
        ProjectGroup::Archived => 3,
    }
}

/// Sorts ascending into "most recent first, absent last": a present timestamp
/// sorts by its negation (larger `t` -> smaller key -> earlier), and `None`
/// gets a key strictly greater than every present one.
fn recency_key(ts: Option<i64>) -> (u8, i64) {
    match ts {
        Some(t) => (0, -t),
        None => (1, 0),
    }
}

/// One project in full: its note, totals, a `settings.chart_days`-long
/// sparkline scoped to this project alone, and every session ever recorded for
/// it. That is the same setting `ui::usage`'s daily chart spans: the two were
/// separate constants that both happened to be 14, and a user who widened one
/// would have been left comparing two different date ranges.
pub fn build_project_detail(
    db: &Db,
    project_id: i64,
    live: &[LiveSession],
    now_ms: i64,
    settings: &Settings,
) -> anyhow::Result<ProjectDetail> {
    let meta = db.project_meta(project_id)?;
    let summary = query::project_summaries(db)?
        .into_iter()
        .find(|s| s.id == project_id)
        .ok_or_else(|| anyhow::anyhow!("no project with id {project_id}"))?;

    let name = meta
        .display_name
        .clone()
        .unwrap_or_else(|| dir_name(&summary.real_path));

    let chart_days = settings.chart_days as usize;
    let days = query::daily_usage_for_project(db, project_id, chart_days, now_ms)?;
    let sparkline = spark_points(
        &days
            .iter()
            .map(|d| d.usage.total_tokens())
            .collect::<Vec<_>>(),
    );

    let sessions = query::session_history(db, project_id)?
        .into_iter()
        .map(|h| {
            let is_live = live.iter().any(|l| l.session_id == h.id);
            let tokens = human_tokens(h.usage.total_tokens());
            // Hidden means absent, not "$0.00" — same rule as the project
            // row above.
            let cost = if settings.show_cost {
                human_cost(h.cost_usd)
            } else {
                String::new()
            };
            let started = since(now_ms, h.started_at);
            let duration = match (h.started_at, h.last_activity_at) {
                (Some(a), Some(b)) if b >= a => human_elapsed(b - a),
                _ => "—".to_string(),
            };
            let mut parts = vec![tokens.clone()];
            if !cost.is_empty() {
                parts.push(cost.clone());
            }
            if duration != "—" {
                parts.push(format!("{duration} long"));
            }
            if let Some(b) = &h.git_branch {
                parts.push(b.clone());
            }
            SessionHistoryRow {
                // The transcript's own title, when Claude Code has written
                // one; otherwise the 8-char UUID prefix history rows have
                // always shown.
                name: h
                    .title
                    .clone()
                    .unwrap_or_else(|| h.id.chars().take(8).collect()),
                detail_line: parts.join(" · "),
                id: h.id,
                started,
                duration,
                tokens,
                cost,
                branch: h.git_branch,
                is_live,
            }
        })
        .collect();

    Ok(ProjectDetail {
        id: project_id,
        path_exists: std::path::Path::new(&summary.real_path).is_dir(),
        path: summary.real_path,
        name,
        note: meta.note.unwrap_or_default(),
        tokens: human_tokens(summary.usage.total_tokens()),
        // Hidden means absent, not "$0.00" — same rule as the project row
        // and the session history row above.
        cost: if settings.show_cost {
            human_cost(summary.cost_usd)
        } else {
            String::new()
        },
        session_count: plural(summary.sessions, "session", "sessions"),
        sparkline,
        sessions,
        pinned: meta.pinned,
        archived: meta.archived,
        notify: meta.notify,
        notify_default_label: default_notify_label(settings.waiting_after_minutes),
        notify_default_minutes: settings.waiting_after_minutes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn, TurnUsage};
    use crate::pricing::seed_default_prices;

    const DAY: i64 = 86_400_000;

    fn project_with_session(
        db: &Db,
        slug: &str,
        path: &str,
        sid: &str,
        last: i64,
        input: u64,
    ) -> i64 {
        let pid = db.upsert_project(slug, path, false).unwrap();
        db.upsert_session(&SessionRecord {
            id: sid.into(),
            project_id: pid,
            file_path: format!("/tmp/{sid}.jsonl"),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(last - 60_000),
            last_activity_at: Some(last),
            cwd: Some(path.into()),
            git_branch: Some("main".into()),
            cc_version: None,
            title: None,
            message_count: 3,
        })
        .unwrap();
        db.insert_turns(
            sid,
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
        pid
    }

    #[test]
    fn projects_are_grouped_with_archived_winning_over_pinned() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let fresh = project_with_session(&db, "-a-fresh", "/a/fresh", "s1", now - DAY, 1_000);
        let stale = project_with_session(&db, "-a-stale", "/a/stale", "s2", now - 30 * DAY, 1_000);
        let pinned = project_with_session(&db, "-a-pin", "/a/pin", "s3", now - 40 * DAY, 1_000);
        let arch = project_with_session(&db, "-a-arch", "/a/arch", "s4", now - DAY, 1_000);
        db.set_pinned(pinned, true).unwrap();
        db.set_pinned(arch, true).unwrap();
        db.set_archived(arch, true).unwrap();

        let m = build_main_window(
            Some(&db),
            PopoverModel::empty(),
            &[],
            now,
            &Settings::default(),
        );
        let group_of = |id: i64| m.projects.iter().find(|p| p.id == id).unwrap().group;
        assert_eq!(group_of(pinned), ProjectGroup::Pinned);
        assert_eq!(
            group_of(fresh),
            ProjectGroup::Active,
            "activity within 7 days"
        );
        assert_eq!(group_of(stale), ProjectGroup::Recent);
        assert_eq!(
            group_of(arch),
            ProjectGroup::Archived,
            "archived beats pinned"
        );
    }

    /// "Show archived projects" is a grouping decision, so it is made here and
    /// not in a shell: with it off, an archived project leaves the sidebar
    /// entirely rather than reappearing under Pinned or Recent.
    #[test]
    fn hiding_archived_projects_drops_them_rather_than_regrouping_them() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let live_one = project_with_session(&db, "-a-fresh", "/a/fresh", "s1", now - DAY, 1_000);
        let arch = project_with_session(&db, "-a-arch", "/a/arch", "s4", now - DAY, 1_000);
        db.set_pinned(arch, true).unwrap();
        db.set_archived(arch, true).unwrap();

        let settings = Settings {
            show_archived: false,
            ..Default::default()
        };
        let m = build_main_window(Some(&db), PopoverModel::empty(), &[], now, &settings);
        assert!(
            m.projects.iter().all(|p| p.id != arch),
            "an archived project is gone when the user hid archived projects, not moved: {:?}",
            m.projects
                .iter()
                .map(|p| (p.id, p.group))
                .collect::<Vec<_>>()
        );
        assert!(
            m.projects.iter().all(|p| p.group != ProjectGroup::Archived),
            "no Archived group is left for a shell to draw"
        );
        assert!(
            m.projects.iter().any(|p| p.id == live_one),
            "every other project is untouched"
        );
    }

    /// A project's Active/Recent boundary is the user's `active_within_days`,
    /// not a frozen seven-day window: the same project falls on either side of
    /// it depending only on the setting.
    #[test]
    fn a_project_is_active_within_its_configured_window() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", now - 10 * DAY, 1_000);
        let group_of = |days: u32| {
            let s = Settings {
                active_within_days: days,
                ..Settings::default()
            };
            build_main_window(Some(&db), PopoverModel::empty(), &[], now, &s)
                .projects
                .iter()
                .find(|p| p.id == pid)
                .unwrap()
                .group
        };

        assert_eq!(
            group_of(7),
            ProjectGroup::Recent,
            "ten days is outside seven"
        );
        assert_eq!(
            group_of(14),
            ProjectGroup::Active,
            "the very same project is inside fourteen"
        );
    }

    /// The defect this prevents: `CHART_DAYS` and `SPARK_DAYS` were two
    /// independent declarations that both happened to be 14, so exposing
    /// either alone would let the usage chart and a project's sparkline
    /// silently disagree about their own date range.
    #[test]
    fn one_setting_drives_both_the_chart_and_the_sparkline() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY + 3_600_000;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", now - 1000, 1_000_000);

        for days in [7u32, 30, 90] {
            let s = Settings {
                chart_days: days,
                ..Settings::default()
            };
            let want = days as usize;
            assert_eq!(
                crate::ui::usage::build_usage(&db, now, &s)
                    .unwrap()
                    .daily
                    .len(),
                want,
                "the usage chart must span chart_days"
            );
            assert_eq!(
                build_project_detail(&db, pid, &[], now, &s)
                    .unwrap()
                    .sparkline
                    .len(),
                want,
                "the sparkline must span the very same setting"
            );
        }
    }

    #[test]
    fn the_sidebar_order_is_decided_here_not_left_to_a_shell() {
        // Group order (Pinned, Active, Recent, Archived) always wins over
        // recency: a pinned project idle for weeks still outranks a very
        // recently active, unpinned one. Within a group, more recent activity
        // sorts first, and a project with no recorded activity sorts last.
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let very_active = project_with_session(&db, "-a-va", "/a/va", "s1", now - 1000, 1_000);
        let pinned = project_with_session(&db, "-a-pin", "/a/pin", "s2", now - 40 * DAY, 1_000);
        let recent_new = project_with_session(&db, "-a-rn", "/a/rn", "s3", now - 10 * DAY, 1_000);
        let recent_old = project_with_session(&db, "-a-ro", "/a/ro", "s4", now - 20 * DAY, 1_000);
        db.set_pinned(pinned, true).unwrap();
        // No turns/no activity at all: sessions exist but `last_activity_at`
        // stays `None` for this project, and it must sort last within `Recent`.
        let recent_silent = db.upsert_project("-a-rs", "/a/rs", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: "s5".into(),
            project_id: recent_silent,
            file_path: "/tmp/s5.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: None,
            last_activity_at: None,
            cwd: Some("/a/rs".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 0,
        })
        .unwrap();

        let m = build_main_window(
            Some(&db),
            PopoverModel::empty(),
            &[],
            now,
            &Settings::default(),
        );
        let ids: Vec<i64> = m.projects.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            vec![pinned, very_active, recent_new, recent_old, recent_silent],
            "pinned first despite being stale; Active before Recent; \
             Recent ordered newest-first with the silent one last"
        );
    }

    #[test]
    fn a_project_row_carries_finished_strings() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        project_with_session(&db, "-a-p", "/a/proj", "s1", now - 3_600_000, 2_000_000);

        let m = build_main_window(
            Some(&db),
            PopoverModel::empty(),
            &[],
            now,
            &Settings::default(),
        );
        let row = &m.projects[0];
        assert_eq!(row.name, "proj", "directory name, not the slug");
        assert_eq!(row.tokens, "2.0M");
        assert_eq!(row.cost, "$30.00");
        assert_eq!(row.session_count, "1 session", "singular");
        assert_eq!(row.last_active, "1h");
        assert!(
            row.subtitle.contains("1 session"),
            "subtitle is composed in Rust"
        );
    }

    #[test]
    fn session_count_is_pluralised_in_rust() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", now - 1000, 10);
        db.upsert_session(&SessionRecord {
            id: "s2".into(),
            project_id: pid,
            file_path: "/tmp/s2.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(now - 2000),
            last_activity_at: Some(now - 2000),
            cwd: Some("/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 1,
        })
        .unwrap();
        let m = build_main_window(
            Some(&db),
            PopoverModel::empty(),
            &[],
            now,
            &Settings::default(),
        );
        assert_eq!(m.projects[0].session_count, "2 sessions");
    }

    #[test]
    fn no_database_yields_an_empty_list_and_no_panic() {
        let m = build_main_window(
            None,
            PopoverModel::empty(),
            &[],
            100 * DAY,
            &Settings::default(),
        );
        assert!(m.projects.is_empty());
        assert!(m.error.is_some(), "the user is told why the list is empty");
    }

    #[test]
    fn project_detail_has_a_full_fourteen_day_sparkline_even_when_quiet() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        // Mid-day, not a day boundary: `100 * DAY` is exactly midnight, so an
        // activity timestamp `1000`ms earlier would land in the *previous*
        // calendar day and this test would wrongly see "today" as quiet.
        let now = 100 * DAY + 3_600_000;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", now - 1000, 1_000_000);

        let d = build_project_detail(&db, pid, &[], now, &Settings::default()).unwrap();
        assert_eq!(d.sparkline.len(), 14);
        assert_eq!(
            d.sparkline[0].day_index, 0,
            "indices are 0..13, oldest first"
        );
        assert_eq!(d.sparkline[13].day_index, 13);
        assert!(d.sparkline[13].tokens > 0, "today has the activity");
        assert!(d.sparkline[0].tokens == 0, "thirteen days ago was quiet");
        assert_eq!(
            d.sparkline[0].label, "13d",
            "oldest day reads as a day count, not \"313h ago\""
        );
        assert_eq!(
            d.sparkline[13].label, "Today",
            "newest day reads as Today, matching ui::usage's scheme"
        );
    }

    #[test]
    fn project_detail_sparkline_is_scoped_to_its_own_project() {
        // A second, noisy project must not bleed into the first project's
        // sparkline — otherwise the chart on a quiet project's page would show
        // someone else's activity as if it were its own.
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let quiet = project_with_session(&db, "-a-quiet", "/a/quiet", "s1", now - 1000, 0);
        project_with_session(&db, "-a-noisy", "/a/noisy", "s2", now - 1000, 5_000_000);

        let d = build_project_detail(&db, quiet, &[], now, &Settings::default()).unwrap();
        assert_eq!(
            d.sparkline[13].tokens, 0,
            "the other project's tokens must not appear on this one's sparkline"
        );
    }

    #[test]
    fn project_detail_lists_sessions_newest_first_and_marks_live_ones() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "old", now - 5 * DAY, 1_000);
        db.upsert_session(&SessionRecord {
            id: "livesess".into(),
            project_id: pid,
            file_path: "/tmp/l.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(now - 1000),
            last_activity_at: Some(now - 500),
            cwd: Some("/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 1,
        })
        .unwrap();

        let live = vec![LiveSession {
            pid: 1,
            session_id: "livesess".into(),
            cwd: "/a/proj".into(),
            name: "n".into(),
            kind: "interactive".into(),
            status: crate::live::SessionStatus::Working,
            started_at: now - 1000,
            status_updated_at: now - 500,
            cc_version: None,
            socket_path: None,
        }];
        let d = build_project_detail(&db, pid, &live, now, &Settings::default()).unwrap();
        assert_eq!(d.sessions[0].id, "livesess");
        assert!(d.sessions[0].is_live);
        assert!(!d.sessions[1].is_live, "the older one has ended");
        assert_eq!(d.sessions[1].branch.as_deref(), Some("main"));
    }

    #[test]
    fn a_session_row_uses_the_stored_title_as_its_name() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = db.upsert_project("-a-p", "/a/proj", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: "6dda468e-ae88-443b-8bf0-4f97f745b455".into(),
            project_id: pid,
            file_path: "/tmp/s.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(now - 1000),
            last_activity_at: Some(now - 500),
            cwd: Some("/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: Some("Claude projects dashboard".into()),
            message_count: 1,
        })
        .unwrap();

        let d = build_project_detail(&db, pid, &[], now, &Settings::default()).unwrap();
        assert_eq!(d.sessions[0].name, "Claude projects dashboard");
    }

    #[test]
    fn a_session_row_with_no_title_falls_back_to_the_uuid_prefix() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = db.upsert_project("-a-p", "/a/proj", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: "6dda468e-ae88-443b-8bf0-4f97f745b455".into(),
            project_id: pid,
            file_path: "/tmp/s.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: Some(now - 1000),
            last_activity_at: Some(now - 500),
            cwd: Some("/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 1,
        })
        .unwrap();

        let d = build_project_detail(&db, pid, &[], now, &Settings::default()).unwrap();
        assert_eq!(
            d.sessions[0].name, "6dda468e",
            "no title: fall back to the first 8 chars of the session id"
        );
    }

    #[test]
    fn a_session_with_no_start_or_end_omits_the_duration_fragment_entirely() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = db.upsert_project("-a-p", "/a/proj", false).unwrap();
        db.upsert_session(&SessionRecord {
            id: "nodur".into(),
            project_id: pid,
            file_path: "/tmp/nodur.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: None,
            last_activity_at: None,
            cwd: Some("/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: 0,
        })
        .unwrap();

        let d = build_project_detail(&db, pid, &[], now, &Settings::default()).unwrap();
        let row = &d.sessions[0];
        assert_eq!(row.duration, "—");
        assert!(
            !row.detail_line.contains("long"),
            "an unknown duration must not render as \"— long\": {}",
            row.detail_line
        );
    }

    #[test]
    fn project_detail_reports_a_missing_directory_rather_than_failing() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = project_with_session(&db, "-nope", "/definitely/not/here", "s1", 100 * DAY, 1);

        let d = build_project_detail(&db, pid, &[], 100 * DAY, &Settings::default()).unwrap();
        assert!(
            !d.path_exists,
            "a vanished project directory is reported, not fatal"
        );
    }

    #[test]
    fn a_project_left_on_default_shows_the_current_global_threshold() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", 100 * DAY, 1);
        let settings = Settings {
            waiting_after_minutes: 25,
            ..Settings::default()
        };

        let d = build_project_detail(&db, pid, &[], 100 * DAY, &settings).unwrap();
        assert_eq!(
            d.notify,
            NotifyOverride::Default,
            "a project that was never given its own override reads as Default"
        );
        assert!(
            d.notify_default_label.contains("25 minutes"),
            "Default's own label must reflect *this* global setting, not a stale or hardcoded one: {}",
            d.notify_default_label
        );
    }

    #[test]
    fn a_project_on_custom_shows_its_own_threshold_not_the_global_one() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", 100 * DAY, 1);
        db.set_notify_override(pid, &NotifyOverride::Custom { after_minutes: 45 })
            .unwrap();
        // Deliberately different from 45, so the two fields can't be
        // confused for one another below.
        let settings = Settings {
            waiting_after_minutes: 10,
            ..Settings::default()
        };

        let d = build_project_detail(&db, pid, &[], 100 * DAY, &settings).unwrap();
        assert_eq!(
            d.notify,
            NotifyOverride::Custom { after_minutes: 45 },
            "notify carries the project's own override"
        );
        assert!(
            d.notify_default_label.contains("10 minutes"),
            "notify_default_label always reflects the *global* setting, \
             regardless of this project's own override: {}",
            d.notify_default_label
        );
    }

    // The label is for reading, not for parsing. A shell seeding a minute
    // control needs the number itself, and it must be the same number the
    // label was composed from — not a literal of the shell's own.
    #[test]
    fn the_default_threshold_is_carried_as_a_number_beside_its_label() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", 100 * DAY, 1);
        db.set_notify_override(pid, &NotifyOverride::Off).unwrap();
        let settings = Settings {
            waiting_after_minutes: 25,
            ..Settings::default()
        };

        let d = build_project_detail(&db, pid, &[], 100 * DAY, &settings).unwrap();
        assert_eq!(
            d.notify_default_minutes, 25,
            "the number behind notify_default_label, present whatever this \
             project's own override is"
        );

        // Move the global setting: the number must move with it, or a shell
        // seeded from it drifts exactly the way a hardcoded literal does.
        let settings = Settings {
            waiting_after_minutes: 7,
            ..Settings::default()
        };
        let d = build_project_detail(&db, pid, &[], 100 * DAY, &settings).unwrap();
        assert_eq!(d.notify_default_minutes, 7);
        assert!(
            d.notify_default_label.contains("7 minutes"),
            "the number and the label must never disagree: {}",
            d.notify_default_label
        );
    }

    #[test]
    fn hiding_cost_hides_it_in_the_main_window_too() {
        // Task 7 (faa3e8d) wired `show_cost` into the popover and the usage
        // view but left this file's three sites unconverted: the project
        // row, the project summary, and the session history row. Off means
        // absent — a blank string, never a fabricated dollar figure.
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", now - 3_600_000, 2_000_000);
        let settings = Settings {
            show_cost: false,
            ..Settings::default()
        };

        let m = build_main_window(Some(&db), PopoverModel::empty(), &[], now, &settings);
        let row = &m.projects[0];
        assert_eq!(row.cost, "", "show_cost off hides the project row's cost");
        assert!(
            !row.subtitle.contains('$'),
            "the project row subtitle must not leak a cost when hidden: {}",
            row.subtitle
        );

        let d = build_project_detail(&db, pid, &[], now, &settings).unwrap();
        assert_eq!(d.cost, "", "show_cost off hides the project summary's cost");
        let session = &d.sessions[0];
        assert_eq!(
            session.cost, "",
            "show_cost off hides the session row's cost"
        );
        assert!(
            !session.detail_line.contains('$'),
            "the session detail line must not leak a cost when hidden: {}",
            session.detail_line
        );
    }

    #[test]
    fn showing_cost_leaves_the_main_window_unchanged() {
        // The other half of the fix: `show_cost = true` (the default) must
        // reproduce every figure exactly as before, so the fix above cannot
        // have over-reached.
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a-p", "/a/proj", "s1", now - 3_600_000, 2_000_000);
        let settings = Settings::default();
        assert!(settings.show_cost, "default is on");

        let m = build_main_window(Some(&db), PopoverModel::empty(), &[], now, &settings);
        let row = &m.projects[0];
        assert_eq!(row.cost, "$30.00");
        assert!(row.subtitle.contains("$30.00"));

        let d = build_project_detail(&db, pid, &[], now, &settings).unwrap();
        assert_eq!(d.cost, "$30.00");
        assert_eq!(d.sessions[0].cost, "$30.00");
        assert!(d.sessions[0].detail_line.contains("$30.00"));
    }

    fn live_at(cwd: &str, id: &str, status: SessionStatus) -> LiveSession {
        LiveSession {
            pid: 1,
            session_id: id.into(),
            cwd: cwd.into(),
            name: "n".into(),
            kind: "interactive".into(),
            status,
            started_at: 1,
            status_updated_at: 1,
            cc_version: None,
            socket_path: None,
        }
    }

    #[test]
    fn a_card_carries_its_readme_sparkline_branch_and_live_labels() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "# X\n\nDoes a useful thing.").unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        let now = 100 * DAY + 3_600_000;
        let pid = project_with_session(&db, "-x", &path, "s1", now - 60_000, 2_000);
        let quiet = project_with_session(&db, "-q", "/gone/perch/quiet", "s2", now - 60 * DAY, 5);

        let live = vec![
            live_at(&path, "l1", SessionStatus::Working),
            live_at(
                &path,
                "l2",
                SessionStatus::Waiting {
                    reason: None,
                    since_ms: 1,
                },
            ),
        ];
        let m = build_main_window(
            Some(&db),
            PopoverModel::empty(),
            &live,
            now,
            &Settings::default(),
        );
        let card = m.projects.iter().find(|p| p.id == pid).unwrap();

        assert_eq!(card.description.as_deref(), Some("Does a useful thing."));
        assert_eq!(
            card.sparkline.len(),
            Settings::default().chart_days as usize
        );
        assert_eq!(card.sparkline.last().unwrap().tokens, 2_000);
        assert_eq!(card.sparkline.last().unwrap().label, "Today");
        assert_eq!(card.live_label.as_deref(), Some("2 running"));
        assert_eq!(card.waiting_label.as_deref(), Some("1 waiting"));
        assert!(
            card.activity_line.starts_with("main · "),
            "{}",
            card.activity_line
        );
        assert!(
            card.stats_line.starts_with("1 session · "),
            "{}",
            card.stats_line
        );

        let q = m.projects.iter().find(|p| p.id == quiet).unwrap();
        assert_eq!(
            q.description, None,
            "a folder that is gone has no description"
        );
        assert_eq!(q.live_label, None);
        assert_eq!(q.waiting_label, None);
        assert!(
            !q.sparkline.is_empty(),
            "quiet is drawn as zero bars, not nothing"
        );
        assert!(q.sparkline.iter().all(|p| p.tokens == 0));
    }

    #[test]
    fn a_card_with_cost_hidden_has_no_dollars_and_no_stray_separator() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        project_with_session(&db, "-a", "/a/a", "s1", now - DAY, 1_000_000);
        let hidden = Settings {
            show_cost: false,
            ..Settings::default()
        };
        let m = build_main_window(Some(&db), PopoverModel::empty(), &[], now, &hidden);
        assert!(
            !m.projects.is_empty(),
            "the fixture must produce a card to inspect"
        );
        for card in &m.projects {
            assert!(!card.stats_line.contains('$'), "{}", card.stats_line);
            assert!(!card.stats_line.ends_with(" · "), "{}", card.stats_line);
            assert!(!card.stats_line.contains(" ·  · "), "{}", card.stats_line);
        }

        let shown = build_main_window(
            Some(&db),
            PopoverModel::empty(),
            &[],
            now,
            &Settings::default(),
        );
        assert!(
            shown.projects[0].stats_line.contains('$'),
            "cost is there when not hidden"
        );
    }

    #[test]
    fn a_project_that_never_ran_in_git_drops_the_branch_cleanly() {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let now = 100 * DAY;
        let pid = project_with_session(&db, "-a", "/a/a", "s1", now - DAY, 10);
        db.conn()
            .execute("UPDATE sessions SET git_branch = NULL", [])
            .unwrap();
        let m = build_main_window(
            Some(&db),
            PopoverModel::empty(),
            &[],
            now,
            &Settings::default(),
        );
        let card = m.projects.iter().find(|p| p.id == pid).unwrap();
        assert_eq!(
            card.activity_line, card.last_active,
            "recency alone, no separator"
        );
    }
}

#[cfg(test)]
mod custom_notify_label_tests {
    use super::custom_notify_label;

    #[test]
    fn a_single_minute_is_not_pluralized() {
        assert_eq!(custom_notify_label(1), "After 1 minute");
    }

    #[test]
    fn more_than_one_minute_is_pluralized() {
        assert_eq!(custom_notify_label(25), "After 25 minutes");
    }
}
