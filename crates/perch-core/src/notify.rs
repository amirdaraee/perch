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
    /// The indexed project this session's `cwd` belongs to — the id every
    /// other model on this boundary is keyed by, and the only thing in this
    /// payload that identifies a project unambiguously. `project` below is a
    /// directory name, and two projects can share one (`~/work/api` and
    /// `~/personal/api`), so a shell routing a click must use this.
    ///
    /// `None` when the index has never seen that directory: there is no
    /// project to point at, and saying so is better than pointing at some
    /// other project that merely shares a name.
    pub project_id: Option<i64>,
    pub project: String,
    pub title: String,
    pub body: String,
    /// Whether the shell should play the platform's alert sound when it
    /// delivers this one. The decision is made here, where the settings
    /// already are, so a shell never reads `Settings` to decide what to do:
    /// it plays the sound or it does not.
    pub sound: bool,
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
/// `override_for` and `project_id_for` are both lookups keyed on a session's
/// `cwd` — the string the index stores as a project's `real_path`. They stay
/// separate closures rather than one combined lookup because the override
/// decides *whether* to fire and the id only travels with a notification that
/// already has: a caller with no index to consult can pass `|_| None` for the
/// second and still get correct decisions.
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
    project_id_for: &dyn Fn(&str) -> Option<i64>,
    already_notified: &HashSet<Episode>,
    now_ms: i64,
) -> (Vec<Notification>, HashSet<Episode>) {
    let mut notifications = Vec::new();
    // Seeded from what we already knew, not empty: absence from one tick is
    // not evidence a block ended. A `sessions/<pid>.json` that is briefly
    // unreadable or half-written is rejected by `live`, so the session simply
    // vanishes from this slice -- and rebuilding from scratch would forget an
    // alert already delivered and fire it again on the next tick. Only
    // *seeing* a session move on licenses forgetting it (below).
    let mut remembered = already_notified.clone();

    // General rule, and the reason for the ordering below: any exclusion that
    // depends on a MUTABLE SETTING (`waiting_enabled`, a project's `Off`
    // override, `include_background`) must sit *after* the already-notified
    // check, never before it as an early return or short-circuit. Toggling a
    // setting must only ever suppress *new* firings — it must never erase the
    // record of an alert already delivered, or the user can flip a switch off
    // and back on and get a repeat notification for a block that never ended.
    //
    // Only an exclusion that depends on an IMMUTABLE FACT about the session —
    // its `status` not being `Waiting` — may short-circuit before that check,
    // because a session that is not waiting genuinely has no episode to carry
    // forward; there is nothing being suppressed, only something that ended.
    //
    // Symmetrically, every mutable-setting exclusion below must `continue`
    // without inserting into `remembered` when it excludes an episode that
    // has never fired — so re-enabling that setting later still finds a
    // genuinely un-notified episode and delivers the alert, rather than
    // treating "excluded" as equivalent to "already handled."

    for session in live {
        // This session is observable right now, so whatever we remembered
        // about it is superseded by what we can see. Dropping its episodes
        // here keeps at most one per session and lets the arms below restate
        // the current truth.
        remembered.retain(|e| e.session_id != session.session_id);

        let SessionStatus::Waiting { reason, since_ms } = &session.status else {
            // Not waiting (working, idle, ended...): an immutable fact about
            // this observation, not a setting. Any episode we had for this
            // session is over. Nothing carried forward for it.
            continue;
        };

        let episode = Episode {
            session_id: session.session_id.clone(),
            waiting_since: *since_ms,
        };

        if already_notified.contains(&episode) {
            // Already fired for this exact episode: keep remembering it, but
            // don't fire again while it stays the same episode. This holds
            // regardless of any of the mutable-setting exclusions below — see
            // the general rule above.
            remembered.insert(episode);
            continue;
        }

        if !settings.waiting_enabled {
            // Feature off and this episode has never fired: nothing to notify,
            // nothing yet to remember either (only a fired episode is memory).
            continue;
        }

        if session.kind == "bg" && !settings.include_background {
            // Excluded by a mutable setting and this episode has never fired:
            // same shape as the disabled case above — no notification, and
            // nothing enters memory, so turning `include_background` on later
            // can still deliver it.
            continue;
        }

        let Some(threshold_ms) = effective_threshold_ms(settings, override_for(&session.cwd))
        else {
            // `Off` and this episode has never fired: same as above — no
            // notification, and nothing enters memory, so switching the
            // project back to Default later can still deliver it.
            continue;
        };

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
            project_id: project_id_for(&session.cwd),
            project,
            title,
            body,
            sound: settings.sound,
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
    /// For the tests that are about *when* an alert fires rather than what
    /// it points at: no index to consult, so no project id.
    fn no_id() -> impl Fn(&str) -> Option<i64> {
        |_| None
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
            &no_id(),
            &HashSet::new(),
            60 * MIN,
        );
        assert!(n.is_empty());
    }

    /// "Play a sound" is decided here, beside every other notification
    /// setting, and travels on the notification itself — a shell must never
    /// read `Settings` to work out whether to chime.
    #[test]
    fn the_sound_preference_travels_with_the_notification() {
        let fire = |sound: bool| {
            let s = Settings { sound, ..on() };
            let (n, _) = decide(
                &[waiting("a", "/p", 0, "interactive")],
                &s,
                &no_override(),
                &no_id(),
                &HashSet::new(),
                60 * MIN,
            );
            assert_eq!(n.len(), 1, "the alert itself still fires either way");
            n[0].sound
        };
        assert!(fire(true), "sound on: the shell is told to chime");
        assert!(!fire(false), "sound off: the shell is told not to");
    }

    #[test]
    fn nothing_fires_before_the_threshold() {
        let (n, seen) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            9 * MIN,
        );
        assert!(n.is_empty(), "9 minutes is under the 10-minute default");
        assert!(seen.is_empty(), "and nothing is remembered yet");
    }

    #[test]
    fn it_fires_once_at_the_threshold_and_not_again_while_still_waiting() {
        let live = [waiting("a", "/p", 0, "interactive")];
        let (n, seen) = decide(
            &live,
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        assert_eq!(n.len(), 1, "fires when the threshold is crossed");
        assert!(
            n[0].body.contains("dialog open"),
            "the reason is in the message: {}",
            n[0].body
        );

        let (again, seen2) = decide(&live, &on(), &no_override(), &no_id(), &seen, 30 * MIN);
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
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        // It goes back to work: the old episode is forgotten.
        let (_, seen) = decide(
            &[working("a", "/p")],
            &on(),
            &no_override(),
            &no_id(),
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
            &no_id(),
            &seen,
            41 * MIN,
        );
        assert_eq!(n.len(), 1, "a genuinely new block deserves a new alert");
    }

    #[test]
    fn an_episode_survives_a_tick_that_cannot_see_its_session() {
        let (n, seen) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        assert_eq!(n.len(), 1, "the first crossing fires");

        // A tick that sees nothing -- an unreadable or half-written record.
        let (n, seen) = decide(&[], &on(), &no_override(), &no_id(), &seen, 20 * MIN);
        assert!(n.is_empty(), "an empty tick fires nothing by itself");
        assert_eq!(seen.len(), 1, "the delivered alert is still remembered");

        // The record comes back, same block, still waiting since 0.
        let (n, _) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &no_id(),
            &seen,
            30 * MIN,
        );
        assert!(
            n.is_empty(),
            "the block never ended, so it must not alert twice"
        );
    }

    #[test]
    fn an_episode_is_forgotten_once_the_session_is_seen_to_move_on() {
        let (_, seen) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        assert_eq!(seen.len(), 1, "the alert is remembered while it holds");

        // Seeing the session working is what licenses forgetting -- not
        // merely failing to see it at all.
        let (_, seen) = decide(
            &[working("a", "/p")],
            &on(),
            &no_override(),
            &no_id(),
            &seen,
            20 * MIN,
        );
        assert!(seen.is_empty(), "it moved on; the episode is over");
    }

    #[test]
    fn background_sessions_are_excluded_unless_asked_for() {
        let bg = [waiting("a", "/p", 0, "bg")];
        let (n, _) = decide(
            &bg,
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            60 * MIN,
        );
        assert!(n.is_empty(), "a bg session is not waiting on *you*");

        let s = Settings {
            include_background: true,
            ..on()
        };
        let (n, _) = decide(&bg, &s, &no_override(), &no_id(), &HashSet::new(), 60 * MIN);
        assert_eq!(n.len(), 1, "unless you say otherwise");
    }

    // Beyond the brief (review round 3, item 1): `include_background` is a live
    // user setting, same as `waiting_enabled` and a project's `Off` override —
    // the third instance of the same bug. Fire with it on, turn it off (memory
    // must survive), turn it back on with the same session still blocked in the
    // same episode: must not fire again.
    #[test]
    fn toggling_include_background_off_and_on_mid_episode_does_not_refire() {
        let bg = [waiting("a", "/p", 0, "bg")];
        let with_bg = Settings {
            include_background: true,
            ..on()
        };

        // Fires once with include_background on.
        let (n, seen) = decide(
            &bg,
            &with_bg,
            &no_override(),
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        assert_eq!(n.len(), 1);

        // Turned off, session still blocked in the same episode: no firing, and
        // the memory of the already-fired episode must survive.
        let (n2, seen2) = decide(&bg, &on(), &no_override(), &no_id(), &seen, 20 * MIN);
        assert!(n2.is_empty(), "include_background off: nothing fires");
        assert_eq!(
            seen2, seen,
            "toggling include_background off must not discard memory of an already-fired episode"
        );

        // Turned back on, same episode still blocked: must not fire again.
        let (n3, _) = decide(&bg, &with_bg, &no_override(), &no_id(), &seen2, 30 * MIN);
        assert!(
            n3.is_empty(),
            "toggling include_background back on must not resurrect a repeat notification"
        );
    }

    // Beyond the brief (review round 3, item 2): symmetrically, a bg session
    // blocked past the threshold while include_background is off, having never
    // fired, must not enter memory — so turning include_background on later
    // still delivers that genuinely un-notified alert.
    #[test]
    fn enabling_include_background_still_delivers_an_alert_that_was_never_sent() {
        let bg = [waiting("a", "/p", 0, "bg")];

        // Excluded the whole time, well past what the threshold would be: never
        // fires, and never enters memory either.
        let (n, seen) = decide(
            &bg,
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            60 * MIN,
        );
        assert!(
            n.is_empty(),
            "excluded: never fires, no matter how long it's waited"
        );
        assert!(
            seen.is_empty(),
            "excluded: nothing enters memory while excluded"
        );

        // include_background turned on, same session still blocked past the
        // threshold: a genuinely un-notified episode, must fire.
        let with_bg = Settings {
            include_background: true,
            ..on()
        };
        let (n2, _) = decide(&bg, &with_bg, &no_override(), &no_id(), &seen, 70 * MIN);
        assert_eq!(
            n2.len(),
            1,
            "enabling include_background must not silently swallow an alert that was never sent"
        );
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
        let (n, _) = decide(&live, &on(), &off, &no_id(), &HashSet::new(), 60 * MIN);
        assert_eq!(n.len(), 1);
        assert_eq!(
            n[0].session_id, "b",
            "only the project that did not opt out"
        );
    }

    // Beyond the brief (review round 2, item 1): the per-project `Off` override
    // must not erase memory either — the same bug as `waiting_enabled`, one level
    // down. Fire on Default, mute the project (Off), unmute it (Default) while
    // the same session is still continuously blocked in the same episode: it
    // must not fire again, and the memory must survive the muted call untouched.
    #[test]
    fn muting_and_unmuting_a_project_mid_episode_does_not_refire() {
        let live = [waiting("a", "/noisy", 0, "interactive")];

        // Fires once on Default.
        let (n, seen) = decide(
            &live,
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        assert_eq!(n.len(), 1);

        // Muted, session still blocked in the same episode: no firing, and the
        // memory of the already-fired episode must survive the muted call.
        let off = |_: &str| NotifyOverride::Off;
        let (n2, seen2) = decide(&live, &on(), &off, &no_id(), &seen, 20 * MIN);
        assert!(n2.is_empty(), "muted: nothing fires");
        assert_eq!(
            seen2, seen,
            "muting must not discard memory of an already-fired episode"
        );

        // Unmuted, same episode still blocked: must not fire again.
        let (n3, _) = decide(&live, &on(), &no_override(), &no_id(), &seen2, 30 * MIN);
        assert!(
            n3.is_empty(),
            "unmuting must not resurrect a repeat notification for the same episode"
        );
    }

    // Beyond the brief (review round 2, item 2): symmetrically, a project set to
    // Off must not swallow an alert that was never delivered. A session blocked
    // past the threshold while its project is Off must not enter memory; setting
    // the project back to Default must then fire for that genuinely un-notified
    // block, not silently stay quiet because Off had "already handled" it.
    #[test]
    fn unmuting_a_project_still_delivers_an_alert_that_was_never_sent() {
        let live = [waiting("a", "/muted", 0, "interactive")];
        let off = |_: &str| NotifyOverride::Off;

        // Off the whole time, well past what the threshold would be: never fires,
        // and — critically — never enters memory either.
        let (n, seen) = decide(&live, &on(), &off, &no_id(), &HashSet::new(), 60 * MIN);
        assert!(
            n.is_empty(),
            "Off: never fires, no matter how long it's waited"
        );
        assert!(seen.is_empty(), "Off: nothing enters memory while muted");

        // Switched back to Default, same session still blocked past the (global)
        // threshold: this is a genuinely un-notified episode and must fire.
        let (n2, _) = decide(&live, &on(), &no_override(), &no_id(), &seen, 70 * MIN);
        assert_eq!(
            n2.len(),
            1,
            "unmuting must not silently swallow an alert that was never sent"
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
        let (n, _) = decide(&live, &on(), &custom, &no_id(), &HashSet::new(), 20 * MIN);
        assert!(n.is_empty(), "20 minutes is under this project's own 45");
        let (n, _) = decide(&live, &on(), &custom, &no_id(), &HashSet::new(), 46 * MIN);
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn the_message_is_finished_in_rust() {
        let (n, _) = decide(
            &[waiting("a", "/Users/x/my-proj", 0, "interactive")],
            &on(),
            &no_override(),
            &no_id(),
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

    // Beyond the brief (review round 1, item 1): disabling the feature mid-block
    // must only suppress new firings, not erase memory of episodes already fired
    // for. Otherwise: fire while enabled -> disable -> re-enable while the *same*
    // episode is still continuously waiting -> it fires again, which is exactly
    // the repeat notification this module exists to prevent.
    #[test]
    fn disabling_and_re_enabling_mid_episode_does_not_refire() {
        let live = [waiting("a", "/p", 0, "interactive")];

        // Fires once while enabled.
        let (n, seen) = decide(
            &live,
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        assert_eq!(n.len(), 1);

        // Disabled, session still blocked in the same episode: no firing, and the
        // memory of the already-fired episode must survive the disabled call.
        let off = Settings {
            waiting_enabled: false,
            ..Default::default()
        };
        let (n2, seen2) = decide(&live, &off, &no_override(), &no_id(), &seen, 20 * MIN);
        assert!(n2.is_empty(), "disabled: nothing fires");
        assert_eq!(
            seen2, seen,
            "disabling must not discard memory of an already-fired episode"
        );

        // Re-enabled, same episode still blocked: must not fire again.
        let (n3, _) = decide(&live, &on(), &no_override(), &no_id(), &seen2, 30 * MIN);
        assert!(
            n3.is_empty(),
            "re-enabling must not resurrect a repeat notification for the same episode"
        );
    }

    // Beyond the brief (review round 1, item 2): the episode key must be
    // (session_id, since_ms), not session_id alone. Every other "new episode"
    // test here has an intervening non-Waiting observation (Working, or the
    // session vanishing) between the two blocks. This one has none: the session
    // is reported Waiting on every single call, but Claude Code's own since_ms
    // jumps forward between calls (a missed poll tick, or Perch restarting and
    // re-attaching mid-block with a slightly different reading — either way, no
    // Working/Idle/absent observation ever separates the two stretches). A
    // session_id-only key would treat this as the same episode forever and
    // never fire again; keying on since_ms catches the jump and fires.
    #[test]
    fn a_since_ms_jump_with_no_intervening_non_waiting_observation_is_a_new_episode() {
        let (_, seen) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        assert_eq!(seen.len(), 1);

        // Still Waiting on every observation Perch makes, but since_ms jumped
        // from 0 to 15 minutes with no non-waiting sample in between.
        let (n, seen2) = decide(
            &[waiting("a", "/p", 15 * MIN, "interactive")],
            &on(),
            &no_override(),
            &no_id(),
            &seen,
            26 * MIN,
        );
        assert_eq!(
            n.len(),
            1,
            "a session_id-only key would have missed this: since_ms moved, so it's a new episode"
        );
        assert_ne!(
            seen2, seen,
            "the memory now tracks the new episode's since_ms, not the old one"
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
        let (n, seen) = decide(
            &live,
            &on(),
            &no_override(),
            &no_id(),
            &HashSet::new(),
            10 * MIN,
        );
        // "a" has waited 10 minutes (over threshold), "b" has waited only 5 (under).
        assert_eq!(n.len(), 1, "only the session actually over threshold fires");
        assert_eq!(n[0].session_id, "a");
        assert_eq!(
            seen.len(),
            1,
            "only the episode that actually fired is remembered"
        );

        // Time passes; "b" now crosses its own threshold too, independently of "a".
        let (n2, seen2) = decide(&live, &on(), &no_override(), &no_id(), &seen, 16 * MIN);
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

    // `project` is a directory name and two projects can share one. The id
    // the caller resolves from the session's `cwd` is what a shell routes a
    // click on, so it has to survive into the notification itself.
    #[test]
    fn a_notification_carries_the_project_id_its_cwd_resolves_to() {
        let live = [
            waiting("a", "/work/api", 0, "interactive"),
            waiting("b", "/personal/api", 0, "interactive"),
        ];
        let ids = |cwd: &str| match cwd {
            "/work/api" => Some(7),
            "/personal/api" => Some(9),
            _ => None,
        };
        let (n, _) = decide(
            &live,
            &on(),
            &no_override(),
            &ids,
            &HashSet::new(),
            60 * MIN,
        );
        let mut got: Vec<_> = n
            .iter()
            .map(|x| (x.session_id.as_str(), x.project_id, x.project.as_str()))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![("a", Some(7), "api"), ("b", Some(9), "api")],
            "both read \"api\"; only the id distinguishes them"
        );
    }

    #[test]
    fn an_unresolvable_cwd_carries_no_project_id_at_all() {
        let (n, _) = decide(
            &[waiting("a", "/p", 0, "interactive")],
            &on(),
            &no_override(),
            &|_| None,
            &HashSet::new(),
            60 * MIN,
        );
        assert_eq!(n[0].project_id, None);
    }
}
