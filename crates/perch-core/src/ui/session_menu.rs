//! One session's own menu: the facts about it, and the things a user can do
//! to it. Composed here — titles, labels, values and the sentence explaining
//! a disabled action included — so a shell only lays it out.

use crate::db::Db;
use crate::model::TurnUsage;
use crate::query::{self, SessionActivity};
use crate::settings::Settings;
use crate::ui::format::{human_cost, human_elapsed, human_tokens};
use crate::ui::model::SessionRow;

/// One labelled fact. Both halves are final strings; a shell puts them side
/// by side and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DetailRow {
    pub label: String,
    pub value: String,
}

/// Which action an item is, so a shell can wire it to the one platform call
/// that performs it. The *title* travels beside it: what the item says is
/// composed here like every other string, and only what it does is the
/// shell's to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MenuActionKind {
    Resume,
    Focus,
    RevealFolder,
    CopySessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MenuAction {
    pub kind: MenuActionKind,
    pub title: String,
    pub enabled: bool,
    /// Why it is unavailable — shown rather than leaving a dead item
    /// unexplained. `None` when enabled.
    pub disabled_reason: Option<String>,
}

/// One bar of the session's own activity chart. The value crosses as a
/// number — a shell scales and draws it — and its label crosses finished, the
/// same division `ui::usage`'s daily chart and the project sparkline already
/// make.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ActivityPoint {
    pub index: i32,
    pub tokens: u64,
    /// Where in the session's life this slice falls — "Start", "15m in".
    pub label: String,
}

/// What this session has actually spent: the four billable token classes
/// broken out, the total, the models that spent it when there was more than
/// one, and the shape of it over the session's own lifetime.
///
/// Every field is empty together when the user has turned row usage off. That
/// is the "you asked not to see this" state, and it is deliberately *not* the
/// same as `note` — a sentence is what "there is nothing to show" looks like,
/// and a user who hid these numbers is not being told the index is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SessionUsage {
    /// "Usage", or empty when the whole section is hidden.
    pub heading: String,
    /// Input, Output, Cache read, Cache write, Total — values already
    /// formatted, the total carrying the cost when the user shows costs.
    pub classes: Vec<DetailRow>,
    /// "By model", or empty when there are no per-model rows to head.
    pub models_heading: String,
    /// One row per model, heaviest first — and present only when this session
    /// used more than one. With a single model these rows restate the total,
    /// and a menu is the wrong place for a line that says nothing.
    pub by_model: Vec<DetailRow>,
    /// "Activity over 3h", or empty when there is no chart to caption.
    pub chart_caption: String,
    pub chart: Vec<ActivityPoint>,
    /// The sentence shown in place of all of the above when there is nothing
    /// to show — no turns recorded, or no index to read them from. `None`
    /// whenever there is.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SessionMenu {
    pub detail: Vec<DetailRow>,
    pub usage: SessionUsage,
    pub actions: Vec<MenuAction>,
    /// Never drawn: the directory [`MenuActionKind::Resume`] runs in and
    /// [`MenuActionKind::RevealFolder`] selects. The *drawn* folder is a
    /// `DetailRow`, which is where the em dash for "not recorded" lives; this
    /// is the raw path, and it is empty exactly when there is none.
    pub folder_path: String,
    /// Never drawn either: what [`MenuActionKind::CopySessionId`] puts on the
    /// pasteboard, and the id [`MenuActionKind::Resume`] resumes.
    pub session_id: String,
}

/// What this menu is built from. The row carries everything already shaped by
/// the user's display preferences; the rest is what a row does not hold.
pub struct SessionContext<'a> {
    pub row: &'a SessionRow,
    /// The session's working directory as recorded, whatever
    /// `show_row_folder` says. That preference is worded for the popover row
    /// ("Show each session's folder underneath its name"), and Resume has to
    /// know where to run regardless of what the row draws.
    pub cwd: &'a str,
    /// The branch the index last saw for this session; `None` when it has
    /// never seen one, or when the index could not be read.
    pub branch: Option<&'a str>,
    /// The application that owns `row.pid`, named by the shell — resolving a
    /// pid to a running application is the one genuinely platform-specific
    /// fact in this menu, so the shell does it and Rust phrases the result.
    /// `None` when nothing could be resolved, which is what disables Focus.
    pub owning_app: Option<&'a str>,
    /// The index, when it opened. `None` — or a query that fails against it —
    /// is a different fact from "this session has no turns yet", and the
    /// usage section says which of the two it is rather than drawing zeroes
    /// for either.
    pub db: Option<&'a Db>,
    /// Read for `show_row_usage` and `show_cost` alone, and only by the usage
    /// section: a four-way token breakdown cannot be carried on the row's two
    /// finished strings the way `Tokens` and `Cost` are. Every other fact in
    /// this menu still reaches it by way of `ctx.row`, and the two are held
    /// together by a test that hiding usage empties both at once.
    pub settings: &'a Settings,
}

const DASH: &str = "—";

/// `value` is pushed only when it holds something. An empty string is the
/// marker for "the user hid this" (`show_row_usage`, `show_cost`), and a
/// hidden fact is not a row with nothing after its label — it is no row at
/// all. An em dash is a value like any other and is always pushed: "Perch
/// does not know" is worth a line.
fn push(out: &mut Vec<DetailRow>, label: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    out.push(DetailRow {
        label: label.to_string(),
        value: value.to_string(),
    });
}

/// An em dash when the string is empty, the string otherwise. For facts that
/// have no user-facing toggle: blank can only mean "not known" there.
fn or_dash(s: &str) -> &str {
    if s.is_empty() {
        DASH
    } else {
        s
    }
}

/// How many bars the activity chart is drawn from. Enough to show a shape —
/// a warm-up, a long quiet stretch, a burst at the end — and few enough that
/// each one is still a readable bar inside a menu.
const ACTIVITY_BUCKETS: usize = 16;

/// The detail rows, the usage section and the actions for one live session.
///
/// Every *metadata* fact reaches this through `ctx.row`, already shaped by the
/// user's display preferences in `build_model` — which is deliberate: the
/// submenu and the row it hangs off can never disagree about whether the user
/// asked to see a number, because only one of them ever decided. The usage
/// section is the one part that reads the index itself, because a breakdown
/// into four token classes and a chart cannot be carried on the row's two
/// finished strings; it applies the very same two preferences, and a test
/// holds the two readings together.
pub fn build_session_menu(ctx: &SessionContext) -> SessionMenu {
    SessionMenu {
        detail: detail_rows(ctx),
        usage: usage_section(ctx),
        actions: actions(ctx),
        folder_path: ctx.cwd.to_string(),
        session_id: ctx.row.id.clone(),
    }
}

/// The value for one token-class row. A class that summed to zero across
/// turns Perch really did read is a *measured* zero, and showing it is honest
/// — it is a fabricated zero, standing in for something unknown, that this
/// codebase never emits.
fn class_rows(usage: &TurnUsage, total_value: String) -> Vec<DetailRow> {
    let mut out = Vec::new();
    for (label, n) in [
        ("Input", usage.input),
        ("Output", usage.output),
        ("Cache read", usage.cache_read),
        ("Cache write", usage.cache_write_total()),
    ] {
        push(&mut out, label, &human_tokens(n));
    }
    push(&mut out, "Total", &total_value);
    out
}

/// "2.0M" on its own, or "2.0M · $3.10" when the user shows costs. The same
/// separator and the same order the popover row composes its own usage
/// segment with, because they are the same two numbers.
fn tokens_and_cost(tokens: u64, cost_usd: f64, show_cost: bool) -> String {
    let tokens = human_tokens(tokens);
    if show_cost {
        format!("{tokens} · {}", human_cost(cost_usd))
    } else {
        tokens
    }
}

fn activity_points(activity: &SessionActivity) -> (String, Vec<ActivityPoint>) {
    let span = activity.last_ts - activity.first_ts;
    // Nothing to plot across: a session whose turns share one instant has no
    // shape, and fifteen empty bars beside one full one would invent one.
    if span <= 0 {
        return (String::new(), Vec::new());
    }
    let n = activity.buckets.len();
    let points = activity
        .buckets
        .iter()
        .enumerate()
        .map(|(i, u)| ActivityPoint {
            index: i as i32,
            tokens: u.total_tokens(),
            // Where in the session's own life this slice falls — not a wall
            // clock, which would need a timezone this layer does not have,
            // and not "ago", which the popover row's `Time in status`
            // already means.
            label: if i == 0 {
                "Start".to_string()
            } else {
                format!("{} in", human_elapsed(span * i as i64 / n as i64))
            },
        })
        .collect();
    (format!("Activity over {}", human_elapsed(span)), points)
}

/// The usage section: the class breakdown, the per-model rows when they say
/// something the total does not, and the session's own activity chart.
fn usage_section(ctx: &SessionContext) -> SessionUsage {
    // Hidden means every field empty and *no* sentence. "You asked not to see
    // this" is not something to explain back to the user who asked for it.
    if !ctx.settings.show_row_usage {
        return SessionUsage::default();
    }
    let heading = "Usage".to_string();
    let nothing = |note: &str| SessionUsage {
        heading: heading.clone(),
        note: Some(note.to_string()),
        ..SessionUsage::default()
    };

    let Some(db) = ctx.db else {
        return nothing("Perch could not read its index, so this session's usage is unknown.");
    };
    let id = &ctx.row.id;
    let (Ok((totals, cost)), Ok(by_model), Ok(activity)) = (
        query::session_usage(db, id),
        query::session_usage_by_model(db, id),
        query::session_activity(db, id, ACTIVITY_BUCKETS),
    ) else {
        return nothing("Perch could not read its index, so this session's usage is unknown.");
    };
    // `session_activity` is `None` for a session the index has never seen a
    // turn for — which is the one case that deserves a sentence rather than a
    // breakdown of zeroes.
    let Some(activity) = activity.filter(|_| totals.total_tokens() > 0) else {
        return nothing("No turns recorded for this session yet.");
    };

    let show_cost = ctx.settings.show_cost;
    let (chart_caption, chart) = activity_points(&activity);
    // One model restates the total; the rows only earn their space when there
    // is something to compare.
    let by_model: Vec<DetailRow> = if by_model.len() > 1 {
        by_model
            .iter()
            .map(|(model, u, c)| DetailRow {
                label: model.clone(),
                value: tokens_and_cost(u.total_tokens(), *c, show_cost),
            })
            .collect()
    } else {
        Vec::new()
    };

    SessionUsage {
        heading,
        classes: class_rows(
            &totals,
            tokens_and_cost(totals.total_tokens(), cost, show_cost),
        ),
        models_heading: if by_model.is_empty() {
            String::new()
        } else {
            "By model".to_string()
        },
        by_model,
        chart_caption,
        chart,
        note: None,
    }
}

fn detail_rows(ctx: &SessionContext) -> Vec<DetailRow> {
    let row = ctx.row;
    let mut out = Vec::new();
    push(&mut out, "Project", or_dash(&row.project));
    push(&mut out, "Folder", or_dash(ctx.cwd));
    push(&mut out, "Branch", ctx.branch.unwrap_or(DASH));
    push(&mut out, "Status", or_dash(&row.status_label));
    // `elapsed` is already an em dash when the session carries no timestamp
    // to measure from (`elapsed_or_dash`), so it needs no second guard.
    push(&mut out, "Time in status", or_dash(&row.elapsed));
    // The two usage rows are the row's own strings, untouched: blank where
    // the user turned the numbers off (and so absent below), an em dash where
    // the index has nothing, never a fabricated zero.
    push(&mut out, "Tokens", &row.tokens);
    push(&mut out, "Cost", &row.cost);
    push(
        &mut out,
        "Claude Code",
        &if row.version.is_empty() {
            DASH.to_string()
        } else {
            format!("v{}", row.version)
        },
    );
    push(&mut out, "Process", &row.pid.to_string());
    push(&mut out, "Session", or_dash(&row.id));
    out
}

fn actions(ctx: &SessionContext) -> Vec<MenuAction> {
    // A real directory, checked now rather than remembered: a worktree can be
    // deleted while its session is still listed, and offering to `cd` into it
    // would just hand the user a terminal that fails. `is_dir` (not `exists`)
    // for the same reason the project pane uses it — a file at that path is
    // not somewhere `claude` can run.
    let folder = match ctx.cwd {
        "" => FolderState::Unrecorded,
        p if std::path::Path::new(p).is_dir() => FolderState::Present,
        p => FolderState::Gone(p),
    };

    vec![
        resume(&folder),
        focus(ctx.owning_app),
        reveal(&folder),
        copy_session_id(&ctx.row.id),
    ]
}

enum FolderState<'a> {
    Present,
    /// The path is recorded but nothing is there any more.
    Gone(&'a str),
    /// No `cwd` ever reached Perch for this session.
    Unrecorded,
}

impl FolderState<'_> {
    fn reason(&self) -> Option<String> {
        match self {
            FolderState::Present => None,
            FolderState::Gone(p) => Some(format!("{p} no longer exists")),
            FolderState::Unrecorded => {
                Some("Perch has no folder recorded for this session".to_string())
            }
        }
    }
}

fn enabled(kind: MenuActionKind, title: &str, reason: Option<String>) -> MenuAction {
    MenuAction {
        kind,
        title: title.to_string(),
        enabled: reason.is_none(),
        disabled_reason: reason,
    }
}

fn resume(folder: &FolderState) -> MenuAction {
    enabled(MenuActionKind::Resume, "Resume", folder.reason())
}

/// Bringing the owning application forward is all this does, and the title
/// says exactly that. Raising the one *window or tab* the session lives in
/// would need the Accessibility permission, which Perch has never asked for
/// — so the title promises the application, never the session.
fn focus(owning_app: Option<&str>) -> MenuAction {
    match owning_app {
        Some(app) => enabled(
            MenuActionKind::Focus,
            &format!("Bring {app} to Front"),
            None,
        ),
        None => enabled(
            MenuActionKind::Focus,
            "Bring to Front",
            Some("Perch could not find the application running this session".to_string()),
        ),
    }
}

fn reveal(folder: &FolderState) -> MenuAction {
    enabled(
        MenuActionKind::RevealFolder,
        "Reveal in Finder",
        folder.reason(),
    )
}

fn copy_session_id(id: &str) -> MenuAction {
    enabled(
        MenuActionKind::CopySessionId,
        "Copy Session ID",
        (id.is_empty()).then(|| "Perch has no session id recorded for this session".to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveSession, SessionStatus};
    use crate::settings::Settings;
    use crate::ui::model::{build_model, Status};

    fn live(cwd: &str, version: Option<&str>) -> LiveSession {
        LiveSession {
            pid: 4242,
            session_id: "0f2c-9a11".into(),
            cwd: cwd.into(),
            name: "perch-0f2c".into(),
            kind: "interactive".into(),
            status: SessionStatus::Working,
            started_at: 1_000,
            status_updated_at: 5_000,
            cc_version: version.map(str::to_string),
            socket_path: None,
        }
    }

    /// One row, built the way the popover builds it — through `build_model`,
    /// so every display preference reaches the submenu by the only route it
    /// can: the row itself.
    fn row_for(session: &LiveSession, settings: &Settings) -> SessionRow {
        row_from(None, session, settings)
    }

    fn row_from(db: Option<&Db>, session: &LiveSession, settings: &Settings) -> SessionRow {
        build_model(db, std::slice::from_ref(session), 65_000, None, settings)
            .live
            .remove(0)
    }

    fn menu(row: &SessionRow, cwd: &str, branch: Option<&str>, app: Option<&str>) -> SessionMenu {
        menu_in(None, &Settings::default(), row, cwd, branch, app)
    }

    fn menu_in(
        db: Option<&Db>,
        settings: &Settings,
        row: &SessionRow,
        cwd: &str,
        branch: Option<&str>,
        app: Option<&str>,
    ) -> SessionMenu {
        build_session_menu(&SessionContext {
            row,
            cwd,
            branch,
            owning_app: app,
            db,
            settings,
        })
    }

    /// The row and the menu built from one index and one `Settings` — the
    /// only combination the app ever builds, since a tick hands both
    /// `build_model` and `build_session_menu` the same pair.
    fn menu_for(db: Option<&Db>, settings: &Settings, session: &LiveSession) -> SessionMenu {
        let row = row_from(db, session, settings);
        menu_in(db, settings, &row, &session.cwd, None, None)
    }

    fn value(m: &SessionMenu, label: &str) -> Option<String> {
        m.detail
            .iter()
            .find(|d| d.label == label)
            .map(|d| d.value.clone())
    }

    fn action(m: &SessionMenu, kind: MenuActionKind) -> &MenuAction {
        m.actions
            .iter()
            .find(|a| a.kind == kind)
            .unwrap_or_else(|| panic!("no {kind:?} action in {:?}", m.actions))
    }

    #[test]
    fn nothing_known_is_an_em_dash_everywhere_and_never_a_zero() {
        // No index (so no usage and no branch), and a session record that
        // never carried a Claude Code version.
        let s = live("/Users/a/proj", None);
        let m = menu(&row_for(&s, &Settings::default()), &s.cwd, None, None);

        assert_eq!(value(&m, "Branch").as_deref(), Some(DASH));
        assert_eq!(value(&m, "Claude Code").as_deref(), Some(DASH));
        assert_eq!(value(&m, "Tokens").as_deref(), Some(DASH));
        assert_eq!(value(&m, "Cost").as_deref(), Some(DASH));
        for d in &m.detail {
            assert_ne!(d.value, "0", "an unknown count must never render as zero");
            assert_ne!(
                d.value, "$0.00",
                "an unknown cost must never render as free"
            );
        }
    }

    #[test]
    fn a_known_version_and_branch_are_shown_as_themselves() {
        let s = live("/Users/a/proj", Some("2.1.251"));
        let m = menu(
            &row_for(&s, &Settings::default()),
            &s.cwd,
            Some("feat/submenu"),
            None,
        );
        assert_eq!(value(&m, "Claude Code").as_deref(), Some("v2.1.251"));
        assert_eq!(value(&m, "Branch").as_deref(), Some("feat/submenu"));
        assert_eq!(value(&m, "Project").as_deref(), Some("proj"));
        assert_eq!(value(&m, "Folder").as_deref(), Some("/Users/a/proj"));
        assert_eq!(value(&m, "Process").as_deref(), Some("4242"));
        assert_eq!(value(&m, "Session").as_deref(), Some("0f2c-9a11"));
    }

    /// The blank marker and the dash marker are different facts, and only the
    /// blank one takes the row away: "you asked not to see this" is answered
    /// by showing nothing at all, never by a label with an empty value.
    #[test]
    fn hiding_usage_takes_both_rows_away_rather_than_dashing_them() {
        let s = live("/Users/a/proj", Some("2.1.251"));
        let settings = Settings {
            show_row_usage: false,
            ..Settings::default()
        };
        let m = menu_in(None, &settings, &row_for(&s, &settings), &s.cwd, None, None);

        assert_eq!(value(&m, "Tokens"), None);
        assert_eq!(value(&m, "Cost"), None);
        assert!(
            value(&m, "Status").is_some(),
            "only the usage rows go; the rest of the menu stays"
        );
    }

    #[test]
    fn hiding_cost_alone_keeps_the_token_row() {
        let s = live("/Users/a/proj", Some("2.1.251"));
        let settings = Settings {
            show_cost: false,
            ..Settings::default()
        };
        let m = menu_in(None, &settings, &row_for(&s, &settings), &s.cwd, None, None);

        assert_eq!(
            value(&m, "Tokens").as_deref(),
            Some(DASH),
            "tokens are still on show; the index simply has none"
        );
        assert_eq!(value(&m, "Cost"), None, "the dollars are what was hidden");
    }

    /// `show_row_folder` is worded for the popover row, and the submenu is
    /// where a user goes to find out *which* checkout this is — so the path
    /// is here either way, and Resume knows where to run either way.
    #[test]
    fn the_folder_is_shown_and_carried_even_when_the_row_hides_it() {
        let s = live("/Users/a/proj", Some("2.1.251"));
        let settings = Settings {
            show_row_folder: false,
            ..Settings::default()
        };
        let row = row_for(&s, &settings);
        assert_eq!(row.folder, "", "the row itself hides it");

        let m = menu(&row, &s.cwd, None, None);
        assert_eq!(value(&m, "Folder").as_deref(), Some("/Users/a/proj"));
        assert_eq!(m.folder_path, "/Users/a/proj");
    }

    #[test]
    fn a_real_directory_enables_resume_and_reveal() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_string_lossy().into_owned();
        let s = live(&cwd, Some("2.1.251"));
        let m = menu(&row_for(&s, &Settings::default()), &cwd, None, None);

        for kind in [MenuActionKind::Resume, MenuActionKind::RevealFolder] {
            let a = action(&m, kind);
            assert!(a.enabled, "{kind:?} should be available: {a:?}");
            assert_eq!(a.disabled_reason, None);
        }
        assert_eq!(action(&m, MenuActionKind::Resume).title, "Resume");
    }

    #[test]
    fn a_folder_that_is_gone_disables_resume_with_a_reason_that_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir
            .path()
            .join("worktree-deleted-yesterday")
            .to_string_lossy()
            .into_owned();
        let s = live(&cwd, Some("2.1.251"));
        let m = menu(&row_for(&s, &Settings::default()), &cwd, None, None);

        let resume = action(&m, MenuActionKind::Resume);
        assert!(!resume.enabled);
        assert_eq!(
            resume.disabled_reason.as_deref(),
            Some(format!("{cwd} no longer exists").as_str()),
        );
        assert_eq!(
            resume.title, "Resume",
            "a disabled action keeps its name — it is explained, not hidden"
        );
        assert!(
            !action(&m, MenuActionKind::RevealFolder).enabled,
            "there is nothing to reveal either"
        );
        assert!(
            action(&m, MenuActionKind::CopySessionId).enabled,
            "the id is still copyable — a missing folder is not a missing session"
        );
    }

    /// A file at the recorded path is not a directory `claude` can run in.
    #[test]
    fn a_file_where_the_folder_used_to_be_is_still_gone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("proj");
        std::fs::write(&file, b"not a directory").unwrap();
        let cwd = file.to_string_lossy().into_owned();
        let s = live(&cwd, None);
        let m = menu(&row_for(&s, &Settings::default()), &cwd, None, None);

        assert!(!action(&m, MenuActionKind::Resume).enabled);
    }

    #[test]
    fn an_unrecorded_folder_gets_its_own_reason() {
        let s = live("", None);
        let m = menu(&row_for(&s, &Settings::default()), "", None, None);

        let resume = action(&m, MenuActionKind::Resume);
        assert!(!resume.enabled);
        assert_eq!(
            resume.disabled_reason.as_deref(),
            Some("Perch has no folder recorded for this session"),
            "an empty path is a different failure from a path that is gone, \
             and saying \" no longer exists\" about nothing would be a lie"
        );
        assert_eq!(value(&m, "Folder").as_deref(), Some(DASH));
    }

    /// Focus promises the application, never the tab: raising one window of a
    /// terminal needs Accessibility, which Perch does not ask for.
    #[test]
    fn focus_names_the_application_it_will_actually_raise() {
        let s = live("/Users/a/proj", None);
        let m = menu(
            &row_for(&s, &Settings::default()),
            &s.cwd,
            None,
            Some("iTerm2"),
        );
        let focus = action(&m, MenuActionKind::Focus);
        assert!(focus.enabled);
        assert_eq!(focus.title, "Bring iTerm2 to Front");
        assert!(
            !focus.title.to_lowercase().contains("session")
                && !focus.title.to_lowercase().contains("tab"),
            "the title must not promise more than an app switch: {}",
            focus.title
        );
    }

    #[test]
    fn focus_is_disabled_with_a_reason_when_no_application_resolves() {
        let s = live("/Users/a/proj", None);
        let m = menu(&row_for(&s, &Settings::default()), &s.cwd, None, None);
        let focus = action(&m, MenuActionKind::Focus);
        assert!(!focus.enabled);
        assert_eq!(
            focus.disabled_reason.as_deref(),
            Some("Perch could not find the application running this session")
        );
    }

    #[test]
    fn every_action_is_present_whatever_is_wrong() {
        let s = live("", None);
        let m = menu(&row_for(&s, &Settings::default()), "", None, None);
        let kinds: Vec<MenuActionKind> = m.actions.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec![
                MenuActionKind::Resume,
                MenuActionKind::Focus,
                MenuActionKind::RevealFolder,
                MenuActionKind::CopySessionId,
            ],
            "an unavailable action is disabled and explained, never dropped"
        );
        assert!(
            m.actions
                .iter()
                .all(|a| a.enabled == a.disabled_reason.is_none()),
            "a disabled action always says why, and an enabled one never does"
        );
    }

    /// Names, paths, statuses, counts and formatted totals — nothing a
    /// session ever *said*. The row model carries no message text in the
    /// first place; this pins the labels so a later row field cannot quietly
    /// bring one in.
    #[test]
    fn the_detail_is_only_metadata() {
        let s = live("/Users/a/proj", Some("2.1.251"));
        let m = menu(
            &row_for(&s, &Settings::default()),
            &s.cwd,
            Some("main"),
            None,
        );
        let labels: Vec<&str> = m.detail.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Project",
                "Folder",
                "Branch",
                "Status",
                "Time in status",
                "Tokens",
                "Cost",
                "Claude Code",
                "Process",
                "Session",
            ]
        );
    }

    #[test]
    fn a_waiting_session_reports_what_it_is_waiting_on() {
        let mut s = live("/Users/a/proj", Some("2.1.251"));
        s.status = SessionStatus::Waiting {
            reason: Some("permission".into()),
            since_ms: 5_000,
        };
        let row = row_for(&s, &Settings::default());
        assert_eq!(row.status, Status::Waiting);
        let m = menu(&row, &s.cwd, None, None);
        assert_eq!(value(&m, "Status").as_deref(), Some("waiting · permission"));
        assert_eq!(
            value(&m, "Time in status").as_deref(),
            Some("1m"),
            "one minute, formatted by the one formatter every shell shares"
        );
    }

    #[test]
    fn the_shell_gets_the_raw_id_and_path_it_has_to_act_on() {
        let s = live("/Users/a/proj", None);
        let m = menu(&row_for(&s, &Settings::default()), &s.cwd, None, None);
        assert_eq!(m.session_id, "0f2c-9a11");
        assert_eq!(m.folder_path, "/Users/a/proj");
    }

    // ---- the usage section -------------------------------------------------

    use crate::db::open_in_memory;
    use crate::model::{SessionRecord, Turn};
    use crate::pricing::seed_default_prices;

    /// An index holding the given turns for the very session `live()` builds,
    /// so the row and its usage section are talking about the same work.
    fn indexed(turns: &[(i64, &str, TurnUsage)]) -> Db {
        let db = open_in_memory().unwrap();
        seed_default_prices(&db).unwrap();
        let pid = db
            .upsert_project("-Users-a-proj", "/Users/a/proj", false)
            .unwrap();
        db.upsert_session(&SessionRecord {
            id: "0f2c-9a11".into(),
            project_id: pid,
            file_path: "/tmp/0f2c-9a11.jsonl".into(),
            file_size: 0,
            indexed_offset: 0,
            started_at: turns.first().map(|t| t.0),
            last_activity_at: turns.last().map(|t| t.0),
            cwd: Some("/Users/a/proj".into()),
            git_branch: None,
            cc_version: None,
            title: None,
            message_count: turns.len() as u64,
        })
        .unwrap();
        let turns: Vec<Turn> = turns
            .iter()
            .map(|(ts, model, usage)| Turn {
                ts: *ts,
                model: (*model).into(),
                usage: *usage,
            })
            .collect();
        db.insert_turns("0f2c-9a11", &turns).unwrap();
        db
    }

    fn spread(input: u64, output: u64, cache_read: u64, cache_write_5m: u64) -> TurnUsage {
        TurnUsage {
            input,
            output,
            cache_read,
            cache_write_5m,
            ..Default::default()
        }
    }

    fn class(u: &SessionUsage, label: &str) -> Option<String> {
        u.classes
            .iter()
            .find(|d| d.label == label)
            .map(|d| d.value.clone())
    }

    /// Every user-visible string the usage section carries, in one list — so
    /// a rule about what may never appear can be checked against all of them
    /// rather than against the handful a test remembered to name.
    fn every_string(u: &SessionUsage) -> Vec<String> {
        let mut out = vec![
            u.heading.clone(),
            u.models_heading.clone(),
            u.chart_caption.clone(),
        ];
        out.extend(u.note.clone());
        for d in u.classes.iter().chain(u.by_model.iter()) {
            out.push(d.label.clone());
            out.push(d.value.clone());
        }
        out.extend(u.chart.iter().map(|p| p.label.clone()));
        out
    }

    #[test]
    fn a_session_with_no_recorded_turns_gets_a_sentence_and_never_a_row_of_zeroes() {
        let s = live("/Users/a/proj", None);
        let db = indexed(&[]);
        let m = menu_for(Some(&db), &Settings::default(), &s);

        assert_eq!(
            m.usage.note.as_deref(),
            Some("No turns recorded for this session yet."),
            "a session Perch has seen no turns for says so in words"
        );
        assert!(
            m.usage.classes.is_empty() && m.usage.chart.is_empty(),
            "a breakdown of zeroes would claim four measurements Perch never made"
        );
        for s in every_string(&m.usage) {
            assert!(
                !s.contains('0') && !s.contains('$'),
                "nothing in the empty state may read as a measured number: {s:?}"
            );
        }
    }

    #[test]
    fn an_index_that_cannot_be_read_says_so_rather_than_claiming_nothing_happened() {
        let s = live("/Users/a/proj", None);
        let m = menu_for(None, &Settings::default(), &s);

        let note = m.usage.note.expect("no index is a fact worth a sentence");
        assert!(
            note.contains("index"),
            "\"could not read\" and \"nothing recorded\" are different facts: {note:?}"
        );
        assert_ne!(
            note, "No turns recorded for this session yet.",
            "an unreadable index must not be reported as a quiet session"
        );
    }

    #[test]
    fn the_four_classes_are_broken_out_and_add_up_to_the_total() {
        let s = live("/Users/a/proj", None);
        // Four deliberately different magnitudes: a breakdown that read the
        // wrong column, or repeated one, cannot produce all four of these.
        let db = indexed(&[
            (
                1_000,
                "claude-fable-5",
                spread(1_000_000, 2_000_000, 3_000_000, 4_000_000),
            ),
            (
                2_000,
                "claude-fable-5",
                spread(1_000_000, 0, 1_000_000, 1_000_000),
            ),
        ]);
        let m = menu_for(Some(&db), &Settings::default(), &s);
        let u = &m.usage;

        assert_eq!(class(u, "Input").as_deref(), Some("2.0M"));
        assert_eq!(class(u, "Output").as_deref(), Some("2.0M"));
        assert_eq!(class(u, "Cache read").as_deref(), Some("4.0M"));
        assert_eq!(class(u, "Cache write").as_deref(), Some("5.0M"));
        // 13M in total, and the row carries the cost beside it.
        let total = class(u, "Total").expect("a breakdown needs its total");
        assert!(
            total.starts_with("13.0M · $"),
            "the total is the four classes summed, with the cost beside it: {total:?}"
        );
        // The same string the popover row shows, composed once: the submenu
        // and the row it hangs off must not disagree about this session.
        assert_eq!(
            total,
            format!("{} · {}", m.detail_value("Tokens"), m.detail_value("Cost")),
        );
    }

    #[test]
    fn one_model_gets_no_per_model_rows_because_they_would_only_restate_the_total() {
        let s = live("/Users/a/proj", None);
        let db = indexed(&[
            (1_000, "claude-fable-5", spread(1_000_000, 0, 0, 0)),
            (2_000, "claude-fable-5", spread(1_000_000, 0, 0, 0)),
        ]);
        let m = menu_for(Some(&db), &Settings::default(), &s);

        assert!(
            m.usage.by_model.is_empty(),
            "one model's row is the total again, in a menu with no room to spare"
        );
        assert_eq!(m.usage.models_heading, "", "and nothing to head it with");
        assert!(
            !m.usage.classes.is_empty(),
            "the breakdown itself still stands"
        );
    }

    #[test]
    fn several_models_each_get_a_row_heaviest_first() {
        let s = live("/Users/a/proj", None);
        // Sonnet inserted first and lighter, so ordering by row order or by
        // name would both put it in front of fable.
        let db = indexed(&[
            (1_000, "claude-sonnet-5", spread(1_000_000, 0, 0, 0)),
            (2_000, "claude-fable-5", spread(2_000_000, 0, 0, 0)),
        ]);
        let m = menu_for(Some(&db), &Settings::default(), &s);

        let labels: Vec<&str> = m.usage.by_model.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(labels, vec!["claude-fable-5", "claude-sonnet-5"]);
        assert_eq!(m.usage.models_heading, "By model");
        // Priced at each model's own rate: fable's 2 MTok at $15.0/MTok and
        // sonnet's 1 MTok at $3.0/MTok. Pooling them under either rate — the
        // bug this guards — gives neither of these.
        assert_eq!(m.usage.by_model[0].value, "2.0M · $30.00");
        assert_eq!(m.usage.by_model[1].value, "1.0M · $3.00");
    }

    #[test]
    fn the_chart_crosses_as_numbers_with_finished_labels_and_spans_the_session() {
        let s = live("/Users/a/proj", None);
        // An hour of work, quiet in the middle: the shape is the point.
        let db = indexed(&[
            (0, "claude-fable-5", spread(1_000, 0, 0, 0)),
            (3_600_000, "claude-fable-5", spread(3_000, 0, 0, 0)),
        ]);
        let m = menu_for(Some(&db), &Settings::default(), &s);
        let u = &m.usage;

        assert_eq!(u.chart.len(), ACTIVITY_BUCKETS);
        assert_eq!(u.chart_caption, "Activity over 1h");
        assert_eq!(u.chart[0].label, "Start");
        assert_eq!(
            u.chart[8].label, "30m in",
            "a bar's caption is finished here, never assembled from its index in a shell"
        );
        assert_eq!(u.chart[0].tokens, 1_000, "values cross as numbers");
        assert_eq!(u.chart[ACTIVITY_BUCKETS - 1].tokens, 3_000);
        assert_eq!(
            u.chart.iter().map(|p| p.tokens).sum::<u64>(),
            4_000,
            "every turn is somewhere on the chart"
        );
        let indices: Vec<i32> = u.chart.iter().map(|p| p.index).collect();
        assert_eq!(indices, (0..ACTIVITY_BUCKETS as i32).collect::<Vec<_>>());
    }

    #[test]
    fn a_session_that_lived_for_one_instant_gets_no_chart_rather_than_an_invented_shape() {
        let s = live("/Users/a/proj", None);
        let db = indexed(&[(7_000, "claude-fable-5", spread(5_000, 0, 0, 0))]);
        let m = menu_for(Some(&db), &Settings::default(), &s);

        assert!(
            m.usage.chart.is_empty() && m.usage.chart_caption.is_empty(),
            "one full bar beside fifteen empty ones is a shape the session never had"
        );
        assert!(
            !m.usage.classes.is_empty(),
            "the breakdown of what it did spend is still known"
        );
    }

    #[test]
    fn hiding_cost_leaves_no_dollars_anywhere_in_the_section() {
        let s = live("/Users/a/proj", None);
        let db = indexed(&[
            (0, "claude-sonnet-5", spread(1_000_000, 0, 0, 0)),
            (3_600_000, "claude-fable-5", spread(2_000_000, 0, 0, 0)),
        ]);
        let settings = Settings {
            show_cost: false,
            ..Settings::default()
        };
        let m = menu_for(Some(&db), &settings, &s);

        // Everything is still on show — the breakdown, both models, the whole
        // chart — so this cannot pass by the section being empty.
        assert_eq!(m.usage.by_model.len(), 2);
        assert_eq!(m.usage.chart.len(), ACTIVITY_BUCKETS);
        assert_eq!(class(&m.usage, "Total").as_deref(), Some("3.0M"));
        for s in every_string(&m.usage) {
            assert!(
                !s.contains('$'),
                "a user who turned cost off must not find dollars here: {s:?}"
            );
        }
    }

    #[test]
    fn hiding_row_usage_empties_the_whole_section_and_says_nothing_about_it() {
        let s = live("/Users/a/proj", None);
        let db = indexed(&[
            (0, "claude-sonnet-5", spread(1_000_000, 0, 0, 0)),
            (3_600_000, "claude-fable-5", spread(2_000_000, 0, 0, 0)),
        ]);
        let shown = menu_for(Some(&db), &Settings::default(), &s);
        assert!(
            !shown.usage.classes.is_empty()
                && !shown.usage.by_model.is_empty()
                && !shown.usage.chart.is_empty(),
            "the fixture must have something to hide for this test to mean anything"
        );

        let settings = Settings {
            show_row_usage: false,
            ..Settings::default()
        };
        let m = menu_for(Some(&db), &settings, &s);

        assert_eq!(
            m.usage,
            SessionUsage::default(),
            "every heading, row, caption and bar goes when the user hides usage"
        );
        assert_eq!(
            m.usage.note, None,
            "\"you asked not to see this\" is not explained back to the user who asked"
        );
        // And the section agrees with the row it hangs under: one preference,
        // two readings of it, never in disagreement.
        assert_eq!(
            m.detail.iter().find(|d| d.label == "Tokens"),
            None,
            "the Tokens row and the usage section hide together or not at all"
        );
    }

    /// The `Tokens`/`Cost` rows the submenu has always pushed really do reach
    /// it when the index has the numbers — the fact the rest of this section
    /// is built on top of.
    #[test]
    fn the_tokens_and_cost_rows_reach_the_submenu_from_a_real_index() {
        let s = live("/Users/a/proj", None);
        let db = indexed(&[(1_000, "claude-fable-5", spread(2_000_000, 0, 0, 0))]);
        let m = menu_for(Some(&db), &Settings::default(), &s);

        assert_eq!(value(&m, "Tokens").as_deref(), Some("2.0M"));
        assert_eq!(value(&m, "Cost").as_deref(), Some("$30.00"));
    }

    impl SessionMenu {
        fn detail_value(&self, label: &str) -> String {
            self.detail
                .iter()
                .find(|d| d.label == label)
                .map(|d| d.value.clone())
                .unwrap_or_default()
        }
    }
}
