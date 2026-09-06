//! Edge-triggered "waiting on you" notification decision.
//!
//! Pure: no I/O, no clock of its own. Same inputs, same answer. The whole value of
//! this feature is firing exactly once, at the moment a session's blocked stretch
//! crosses the threshold — never again while it stays blocked, and never on a level
//! check that would fire on every poll tick.

use crate::db::NotifyOverride;
use crate::live::{LiveSession, SessionStatus};
use crate::settings::Settings;
use crate::ui::format::human_elapsed;
use std::collections::HashSet;

/// One session's blocked stretch. A session that blocks, resumes, and blocks
/// again is a new episode — the key comes from data Claude Code owns, so it
/// survives a Perch restart without re-notifying.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Episode {
    pub session_id: String,
    pub waiting_since: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub session_id: String,
    pub project: String,
    pub title: String,
    pub body: String,
}

/// The directory name Perch shows everywhere: the last path component of `cwd`.
fn project_name(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string())
}

/// Project override beats the global setting. `Off` means never; `Custom { n }`
/// uses `n`; `Default` falls through to the global `waiting_after_minutes`.
fn effective_threshold_ms(settings: &Settings, override_: NotifyOverride) -> Option<i64> {
    match override_ {
        NotifyOverride::Off => None,
        NotifyOverride::Custom { after_minutes } => Some(i64::from(after_minutes) * 60_000),
        NotifyOverride::Default => Some(i64::from(settings.waiting_after_minutes) * 60_000),
    }
}

fn compose(project: &str, reason: Option<&str>, elapsed_ms: i64) -> (String, String) {
    let title = format!("{project} is waiting on you");
    let elapsed = human_elapsed(elapsed_ms);
    let body = match reason {
        Some(reason) => format!("{reason} — waiting {elapsed}"),
        None => format!("Waiting {elapsed}"),
    };
    (title, body)
}

/// Pure: same inputs, same answer. `already_notified` is the caller's memory of
/// episodes it has fired for; the returned set replaces it.
///
/// The returned set contains exactly the episodes that have actually fired (just
/// now, or on a previous call) and are still live and still waiting. A session
/// not yet over its threshold is not remembered — there is nothing to dedup
/// against yet. A session absent from `live`, or no longer `Waiting`, carries
/// nothing forward. That is what keeps the memory bounded and lets a
/// resumed-then-reblocked session read as a genuinely new episode.
pub fn decide(
    live: &[LiveSession],
    settings: &Settings,
    override_for: &dyn Fn(&str) -> NotifyOverride,
    already_notified: &HashSet<Episode>,
    now_ms: i64,
) -> (Vec<Notification>, HashSet<Episode>) {
    let mut notifications = Vec::new();
    let mut remembered = HashSet::new();

    if !settings.waiting_enabled {
        return (notifications, remembered);
    }

    for session in live {
        let SessionStatus::Waiting { reason, since_ms } = &session.status else {
            // Not waiting (working, idle, ended...): any episode we had for this
            // session is over. Nothing carried forward for it.
            continue;
        };

        if session.kind == "bg" && !settings.include_background {
            continue;
        }

        let Some(threshold_ms) = effective_threshold_ms(settings, override_for(&session.cwd))
        else {
            // `Off`: never notify, and never remember, for this project.
            continue;
        };

        let episode = Episode {
            session_id: session.session_id.clone(),
            waiting_since: *since_ms,
        };

        if already_notified.contains(&episode) {
            // Already fired for this exact episode: keep remembering it, but
            // don't fire again while it stays the same episode.
            remembered.insert(episode);
            continue;
        }

        let elapsed_ms = now_ms - since_ms;
        if elapsed_ms < threshold_ms {
            // Still under threshold: not yet notified, so nothing to remember
            // yet either. Only a fired episode is memory.
            continue;
        }

        let project = project_name(&session.cwd);
        let (title, body) = compose(&project, reason.as_deref(), elapsed_ms);
        notifications.push(Notification {
            session_id: session.session_id.clone(),
            project,
            title,
            body,
        });
        remembered.insert(episode);
    }

    (notifications, remembered)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn waiting(id: &str, cwd: &str, since_ms: i64, kind: &str) -> LiveSession {
        LiveSession {
            pid: 1,
            session_id: id.into(),
            cwd: cwd.into(),
            name: id.into(),
            kind: kind.into(),
            status: SessionStatus::Waiting {
                reason: Some("dialog open".into()),
                since_ms,
            },
            started_at: 0,
            status_updated_at: since_ms,
            cc_version: None,
            socket_path: None,
        }
    }
    fn working(id: &str, cwd: &str) -> LiveSession {
        LiveSession {
            status: SessionStatus::Working,
            ..waiting(id, cwd, 0, "interactive")
        }
    }
    fn on() -> Settings {
        Settings {
            waiting_enabled: true,
            ..Default::default()
        }
    }
    fn no_override() -> impl Fn(&str) -> NotifyOverride {
        |_| NotifyOverride::Default
    }

    #[test]
    fn nothing_fires_while_the_feature_is_off() {
        let s = Settings {
            waiting_enabled: false,
            ..Default::default()
        };
        let (n, _) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &s,
            &no_override(),
            &HashSet::new(),
            60 * MIN,
        );
        assert!(n.is_empty());
    }

    #[test]
    fn nothing_fires_before_the_threshold() {
        let (n, seen) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &HashSet::new(),
            9 * MIN,
        );
        assert!(n.is_empty(), "9 minutes is under the 10-minute default");
        assert!(seen.is_empty(), "and nothing is remembered yet");
    }

    #[test]
    fn it_fires_once_at_the_threshold_and_not_again_while_still_waiting() {
        let live = [waiting("a", "/p", 0, "interactive")];
        let (n, seen) = decide(&live, &on(), &no_override(), &HashSet::new(), 10 * MIN);
        assert_eq!(n.len(), 1, "fires when the threshold is crossed");
        assert!(
            n[0].body.contains("dialog open"),
            "the reason is in the message: {}",
            n[0].body
        );

        let (again, seen2) = decide(&live, &on(), &no_override(), &seen, 30 * MIN);
        assert!(
            again.is_empty(),
            "level-triggering here would notify every tick"
        );
        assert_eq!(seen2, seen, "and the memory is unchanged");
    }

    #[test]
    fn a_session_that_resumes_and_blocks_again_is_a_new_episode() {
        let (_, seen) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &HashSet::new(),
            10 * MIN,
        );
        // It goes back to work: the old episode is forgotten.
        let (_, seen) = decide(
            &[working("a", "/p")],
            &on(),
            &no_override(),
            &seen,
            20 * MIN,
        );
        assert!(
            seen.is_empty(),
            "an episode ends when the session works again"
        );
        // Blocks again, later.
        let (n, _) = decide(
            &[waiting("a", "/p", 30 * MIN, "interactive")],
            &on(),
            &no_override(),
            &seen,
            41 * MIN,
        );
        assert_eq!(n.len(), 1, "a genuinely new block deserves a new alert");
    }

    #[test]
    fn a_vanished_session_is_forgotten_so_the_memory_cannot_grow_forever() {
        let (_, seen) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &HashSet::new(),
            10 * MIN,
        );
        let (_, seen) = decide(&[], &on(), &no_override(), &seen, 20 * MIN);
        assert!(
            seen.is_empty(),
            "the session is gone; its episode goes with it"
        );
    }

    #[test]
    fn background_sessions_are_excluded_unless_asked_for() {
        let bg = [waiting("a", "/p", 0, "bg")];
        let (n, _) = decide(&bg, &on(), &no_override(), &HashSet::new(), 60 * MIN);
        assert!(n.is_empty(), "a bg session is not waiting on *you*");

        let s = Settings {
            include_background: true,
            ..on()
        };
        let (n, _) = decide(&bg, &s, &no_override(), &HashSet::new(), 60 * MIN);
        assert_eq!(n.len(), 1, "unless you say otherwise");
    }

    #[test]
    fn a_project_set_to_off_never_notifies() {
        let off = |cwd: &str| {
            if cwd == "/quiet" {
                NotifyOverride::Off
            } else {
                NotifyOverride::Default
            }
        };
        let live = [
            waiting("a", "/quiet", 0, "interactive"),
            waiting("b", "/loud", 0, "interactive"),
        ];
        let (n, _) = decide(&live, &on(), &off, &HashSet::new(), 60 * MIN);
        assert_eq!(n.len(), 1);
        assert_eq!(
            n[0].session_id, "b",
            "only the project that did not opt out"
        );
    }

    #[test]
    fn a_project_with_a_custom_threshold_uses_its_own() {
        let custom = |cwd: &str| {
            if cwd == "/slow" {
                NotifyOverride::Custom { after_minutes: 45 }
            } else {
                NotifyOverride::Default
            }
        };
        let live = [waiting("a", "/slow", 0, "interactive")];
        let (n, _) = decide(&live, &on(), &custom, &HashSet::new(), 20 * MIN);
        assert!(n.is_empty(), "20 minutes is under this project's own 45");
        let (n, _) = decide(&live, &on(), &custom, &HashSet::new(), 46 * MIN);
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn the_message_is_finished_in_rust() {
        let (n, _) = decide(
            &[waiting("a", "/Users/x/my-proj", 0, "interactive")],
            &on(),
            &no_override(),
            &HashSet::new(),
            12 * MIN,
        );
        let m = &n[0];
        assert_eq!(
            m.project, "my-proj",
            "the directory name, as everywhere else"
        );
        assert!(!m.title.is_empty() && !m.body.is_empty());
        assert!(
            m.body.contains("12m") || m.body.contains("12"),
            "how long, already formatted: {}",
            m.body
        );
    }

    // Beyond the brief: two sessions waiting in the same project must be tracked
    // as independent episodes (keyed on session_id, not on cwd/project), each
    // crossing its own threshold on its own schedule. A wrong implementation
    // that grouped by project (or shared one "already notified" flag per cwd)
    // would either fire for both at once or silently drop one.
    #[test]
    fn two_sessions_in_the_same_project_are_independent_episodes() {
        let live = [
            waiting("a", "/p", 0, "interactive"),
            waiting("b", "/p", 5 * MIN, "interactive"),
        ];
        let (n, seen) = decide(&live, &on(), &no_override(), &HashSet::new(), 10 * MIN);
        // "a" has waited 10 minutes (over threshold), "b" has waited only 5 (under).
        assert_eq!(n.len(), 1, "only the session actually over threshold fires");
        assert_eq!(n[0].session_id, "a");
        assert_eq!(
            seen.len(),
            1,
            "only the episode that actually fired is remembered"
        );

        // Time passes; "b" now crosses its own threshold too, independently of "a".
        let (n2, seen2) = decide(&live, &on(), &no_override(), &seen, 16 * MIN);
        assert_eq!(
            n2.len(),
            1,
            "a is not re-notified; b fires on its own schedule"
        );
        assert_eq!(n2[0].session_id, "b");
        assert_eq!(
            seen2.len(),
            2,
            "both episodes are now remembered, having each crossed their own threshold"
        );
    }
}
