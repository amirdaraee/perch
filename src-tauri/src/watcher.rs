use notify::{RecursiveMode, Watcher};
use perch_core::{config, live, platform::RealProcessProbe};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Coalesce bursts: one status change rewrites the record file and can produce
/// several filesystem events.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Re-poll even without an event, so a session whose process dies (leaving a
/// stale record and no filesystem change) still disappears from the UI.
const POLL: Duration = Duration::from_secs(5);

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let Some(cfg) = config::config_dir() else {
            eprintln!("perch: watcher: could not locate a Claude Code config directory");
            return;
        };
        let dir = config::sessions_dir(&cfg);

        let (tx, rx) = mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(err) => {
                eprintln!("perch: watcher: failed to create filesystem watcher: {err}");
                return;
            }
        };
        if let Err(err) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
            eprintln!("perch: watcher: failed to watch {}: {err}", dir.display());
            return;
        }

        emit_now(&app, &dir);
        let mut last = Instant::now();
        // Set when an event arrives inside the debounce dead window and gets
        // dropped: the next wait ends at the debounce boundary instead of the
        // full poll boundary, so that event's change is not held back until
        // the next 5 s poll.
        let mut pending = false;

        loop {
            let wait = if pending {
                DEBOUNCE.saturating_sub(last.elapsed())
            } else {
                POLL
            };
            match rx.recv_timeout(wait) {
                Ok(_) => {
                    if last.elapsed() >= DEBOUNCE {
                        emit_now(&app, &dir);
                        last = Instant::now();
                        pending = false;
                    } else {
                        pending = true;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    emit_now(&app, &dir);
                    last = Instant::now();
                    pending = false;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

fn emit_now(app: &AppHandle, dir: &std::path::Path) {
    let sessions = live::live_sessions(dir, &RealProcessProbe);
    let _ = app.emit("sessions-changed", &sessions);

    // Spec §9.1 asks for a window-utilization percentage, which needs the
    // tier-1 rate-limit endpoint (a later milestone). §8 forbids fabricating
    // a percentage against an unknown ceiling, so show what can be shown
    // honestly: a live-session count, with an hourglass when something is
    // blocked waiting.
    let waiting = sessions
        .iter()
        .filter(|s| matches!(s.status, live::SessionStatus::Waiting { .. }))
        .count();
    let title = if waiting > 0 {
        format!("{} ⏳", waiting)
    } else if sessions.is_empty() {
        String::new()
    } else {
        format!("{}", sessions.len())
    };
    // TrayIcon::set_title dispatches to the main thread internally (via
    // run_on_main_thread, blocking on a channel recv) and is documented as
    // safe to call from any thread, including this watcher thread — no
    // explicit run_on_main_thread wrapping needed here.
    if let Some(tray) = app.tray_by_id("perch-tray") {
        let _ = tray.set_title(Some(title));
    }
}
