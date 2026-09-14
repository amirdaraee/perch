//! One session's own menu: the facts about it, and the things a user can do
//! to it. Composed here — titles, labels, values and the sentence explaining
//! a disabled action included — so a shell only lays it out.

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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SessionMenu {
    pub detail: Vec<DetailRow>,
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

/// The detail rows and the actions for one live session.
///
/// Nothing here reads the index or the settings file: every display
/// preference has already been applied to `ctx.row` by `build_model`, which
/// is deliberate — the submenu and the row it hangs off can never disagree
/// about whether the user asked to see a number, because only one of them
/// ever decided.
pub fn build_session_menu(ctx: &SessionContext) -> SessionMenu {
    SessionMenu {
        detail: detail_rows(ctx),
        actions: actions(ctx),
        folder_path: ctx.cwd.to_string(),
        session_id: ctx.row.id.clone(),
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
        build_model(None, std::slice::from_ref(session), 65_000, None, settings)
            .live
            .remove(0)
    }

    fn menu(row: &SessionRow, cwd: &str, branch: Option<&str>, app: Option<&str>) -> SessionMenu {
        build_session_menu(&SessionContext {
            row,
            cwd,
            branch,
            owning_app: app,
        })
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
        let m = menu(&row_for(&s, &settings), &s.cwd, None, None);

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
        let m = menu(&row_for(&s, &settings), &s.cwd, None, None);

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
}
