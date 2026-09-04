//! Watches the sessions directory and reports the live session list.
//! Moved from the Tauri shell; no UI framework in sight.

use crate::live::{live_sessions, LiveSession};
use crate::platform::ProcessProbe;
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct WatcherConfig {
    pub sessions_dir: PathBuf,
    pub debounce: Duration,
    pub poll: Duration,
}

impl WatcherConfig {
    /// Production defaults: 250 ms coalescing, 5 s backstop.
    pub fn for_dir(sessions_dir: PathBuf) -> Self {
        Self {
            sessions_dir,
            debounce: Duration::from_millis(250),
            poll: Duration::from_secs(5),
        }
    }
}

enum Cmd {
    Refresh,
    Stop,
}

/// Filesystem-change notifications and control commands share one channel so
/// the run loop has exactly one wait point.
enum Event {
    Fs,
    Cmd(Cmd),
}

pub struct WatcherHandle {
    events: mpsc::Sender<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WatcherHandle {
    /// Emit now, regardless of debounce. Used when the menu opens.
    pub fn refresh(&self) {
        let _ = self.events.send(Event::Cmd(Cmd::Refresh));
    }

    /// Stop the watcher thread and block until it has exited.
    pub fn stop(mut self) {
        let _ = self.events.send(Event::Cmd(Cmd::Stop));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Start watching. Emits the current list immediately, then on every change
/// (debounced, trailing-edge) and at least every `poll` as a backstop — a session
/// whose process dies produces no filesystem event, so polling is what removes it.
pub fn spawn<F>(cfg: WatcherConfig, probe: Arc<dyn ProcessProbe>, on_sessions: F) -> WatcherHandle
where
    F: Fn(Vec<LiveSession>) + Send + 'static,
{
    let (ev_tx, ev_rx) = mpsc::channel::<Event>();
    let fs_tx = ev_tx.clone();
    let thread = std::thread::spawn(move || run(cfg, probe, on_sessions, fs_tx, ev_rx));
    WatcherHandle {
        events: ev_tx,
        thread: Some(thread),
    }
}

fn run<F>(
    cfg: WatcherConfig,
    probe: Arc<dyn ProcessProbe>,
    on_sessions: F,
    fs_tx: mpsc::Sender<Event>,
    ev_rx: mpsc::Receiver<Event>,
) where
    F: Fn(Vec<LiveSession>),
{
    let dir = cfg.sessions_dir.clone();
    let emit = |probe: &dyn ProcessProbe| on_sessions(live_sessions(&dir, probe));

    let mut watcher = match notify::recommended_watcher(move |_res| {
        let _ = fs_tx.send(Event::Fs);
    }) {
        Ok(w) => Some(w),
        Err(err) => {
            eprintln!("perch: watcher: failed to create filesystem watcher: {err}");
            None
        }
    };

    // A fresh Claude Code install has no `sessions` directory yet, and notify
    // then fails with `path_not_found`. Fall through to a poll-only loop and
    // retry `watch()` on every tick. The directory is never created here —
    // Perch is strictly read-only with respect to the Claude Code directory.
    let mut watching = try_watch(watcher.as_mut(), &dir, true);

    emit(&*probe);
    let mut last = Instant::now();
    // Set when an event arrives inside the debounce dead window and gets
    // dropped: the next wait ends at the debounce boundary instead of the
    // full poll boundary, so that event's change is not held back until the
    // next poll.
    let mut pending = false;

    loop {
        let wait = if watching && pending {
            cfg.debounce.saturating_sub(last.elapsed())
        } else {
            cfg.poll
        };
        match ev_rx.recv_timeout(wait) {
            Ok(Event::Cmd(Cmd::Stop)) => return,
            Ok(Event::Cmd(Cmd::Refresh)) => {
                emit(&*probe);
                last = Instant::now();
                pending = false;
            }
            Ok(Event::Fs) => {
                if watching && last.elapsed() < cfg.debounce {
                    pending = true;
                } else {
                    emit(&*probe);
                    last = Instant::now();
                    pending = false;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                emit(&*probe);
                last = Instant::now();
                pending = false;
                // Poll-only mode: the directory may have appeared since the
                // last tick. One success promotes us back to the normal
                // debounced loop.
                if !watching {
                    watching = try_watch(watcher.as_mut(), &dir, false);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Never creates the directory: if it is absent, `watch` fails and we poll instead.
fn try_watch(
    watcher: Option<&mut notify::RecommendedWatcher>,
    dir: &Path,
    log_failure: bool,
) -> bool {
    let Some(w) = watcher else { return false };
    match w.watch(dir, RecursiveMode::NonRecursive) {
        Ok(()) => true,
        Err(err) => {
            if log_failure {
                eprintln!(
                    "perch: watcher: failed to watch {}: {err} — polling every {:?} and retrying",
                    dir.display(),
                    Duration::from_secs(5)
                );
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct AllAlive;
    impl ProcessProbe for AllAlive {
        fn is_alive(&self, _pid: i32) -> bool {
            true
        }
        fn process_name(&self, _pid: i32) -> Option<String> {
            Some("claude".into())
        }
    }

    fn write_record(dir: &Path, pid: i32, id: &str) {
        std::fs::write(
            dir.join(format!("{pid}.json")),
            format!(r#"{{"pid":{pid},"sessionId":"{id}","cwd":"/t","name":"n","kind":"interactive","status":"busy","startedAt":1,"statusUpdatedAt":2}}"#),
        ).unwrap();
    }

    fn cfg(dir: &Path) -> WatcherConfig {
        WatcherConfig {
            sessions_dir: dir.to_path_buf(),
            debounce: Duration::from_millis(50),
            poll: Duration::from_millis(300),
        }
    }

    #[test]
    fn emits_once_on_start_and_again_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 1, "one");
        let (tx, rx) = mpsc::channel();
        let h = spawn(cfg(tmp.path()), Arc::new(AllAlive), move |s| {
            let _ = tx.send(s);
        });

        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("initial emit");
        assert_eq!(first.len(), 1);

        write_record(tmp.path(), 2, "two");
        let mut seen: HashSet<usize> = HashSet::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(200)) {
                seen.insert(s.len());
            }
            if seen.contains(&2) {
                break;
            }
        }
        assert!(
            seen.contains(&2),
            "a new record must surface via watch or poll"
        );
        h.stop();
    }

    #[test]
    fn missing_directory_falls_back_to_polling_and_never_creates_it() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("sessions");
        let (tx, rx) = mpsc::channel();
        let h = spawn(cfg(&missing), Arc::new(AllAlive), move |s| {
            let _ = tx.send(s);
        });

        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("poll-only still emits");
        assert!(first.is_empty());
        assert!(
            !missing.exists(),
            "watcher must never create the sessions dir"
        );

        // Directory appears later: the retry should pick up the record within a few polls.
        std::fs::create_dir_all(&missing).unwrap();
        write_record(&missing, 3, "three");
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut got = false;
        while Instant::now() < deadline {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(200)) {
                if s.len() == 1 {
                    got = true;
                    break;
                }
            }
        }
        assert!(got, "record written after the dir appears must surface");
        h.stop();
    }

    #[test]
    fn refresh_forces_an_immediate_emit() {
        let tmp = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let h = spawn(
            WatcherConfig {
                sessions_dir: tmp.path().into(),
                debounce: Duration::from_millis(50),
                poll: Duration::from_secs(30),
            },
            Arc::new(AllAlive),
            move |s| {
                let _ = tx.send(s);
            },
        );
        rx.recv_timeout(Duration::from_secs(2)).expect("initial");
        h.refresh();
        rx.recv_timeout(Duration::from_secs(1))
            .expect("refresh must not wait for the 30s poll");
        h.stop();
    }
}
