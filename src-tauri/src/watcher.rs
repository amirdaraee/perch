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
            return;
        };
        let dir = config::sessions_dir(&cfg);

        let (tx, rx) = mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(_) => return,
        };
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }

        emit_now(&app, &dir);
        let mut last = Instant::now();

        loop {
            match rx.recv_timeout(POLL) {
                Ok(_) => {
                    if last.elapsed() >= DEBOUNCE {
                        emit_now(&app, &dir);
                        last = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    emit_now(&app, &dir);
                    last = Instant::now();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

fn emit_now(app: &AppHandle, dir: &std::path::Path) {
    let sessions = live::live_sessions(dir, &RealProcessProbe);
    let _ = app.emit("sessions-changed", sessions);
}
