//! Watches the sessions directory and reports the live session list.
//! No UI framework in sight.
//!
//! Forked from (not moved from) the Tauri shell's near-identical loop at
//! `src-tauri/src/watcher.rs`, kept deliberately until that app is removed
//! (backlog: "Remove the Tauri app and React frontend"). A debounce or
//! promotion fix here does not reach that copy on its own.

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
    thread_id: std::thread::ThreadId,
}

impl WatcherHandle {
    /// Run `on_refresh` (see `spawn`), then emit, regardless of debounce.
    /// Used when the menu opens. A pure signal: the work happens on the
    /// watcher thread, not the caller's, so a slow `on_refresh` never blocks
    /// whoever calls this (menu-open handlers are commonly on a UI thread).
    pub fn refresh(&self) {
        let _ = self.events.send(Event::Cmd(Cmd::Refresh));
    }

    /// Stop the watcher thread and block until it has exited.
    ///
    /// `on_sessions` runs synchronously on the watcher thread, so calling
    /// `stop()` from inside that callback would otherwise deadlock: this
    /// thread would join itself. That case is detected and tolerated here —
    /// the `Stop` command is still sent (and will be processed once the
    /// callback returns control to the run loop), but the join is skipped
    /// rather than waiting forever. Called from any other thread, `stop()`
    /// blocks until the watcher thread has actually exited.
    pub fn stop(mut self) {
        let _ = self.events.send(Event::Cmd(Cmd::Stop));
        if std::thread::current().id() == self.thread_id {
            return;
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WatcherHandle {
    /// Safety net for a handle dropped without calling `stop()`: the run
    /// loop holds its own clone of this sender (for the filesystem-watch
    /// callback), so the channel never disconnects on its own — without this,
    /// the thread, and the live OS-level notify watch it holds, would leak
    /// forever. `Drop` must not block, so this only asks the thread to exit;
    /// it does not join. Calling `stop()` explicitly remains the way to wait
    /// for a clean, joined shutdown.
    fn drop(&mut self) {
        let _ = self.events.send(Event::Cmd(Cmd::Stop));
    }
}

/// Start watching. Emits the current list immediately, then on every change
/// (debounced, trailing-edge) and at least every `poll` as a backstop — a session
/// whose process dies produces no filesystem event, so polling is what removes it.
///
/// `on_refresh` runs on the watcher thread, only when an explicit
/// `WatcherHandle::refresh()` call is processed — never on the initial emit,
/// a filesystem event, or a poll tick — so a caller (e.g. the FFI layer's
/// index re-index, which is real work: a full directory walk plus SQLite
/// round-trips) never blocks whoever asked for the refresh, and two
/// `refresh()` calls can never run that work concurrently with each other:
/// this thread processes one event at a time.
pub fn spawn<F, R>(
    cfg: WatcherConfig,
    probe: Arc<dyn ProcessProbe>,
    on_sessions: F,
    on_refresh: R,
) -> WatcherHandle
where
    F: Fn(Vec<LiveSession>) + Send + 'static,
    R: Fn() + Send + 'static,
{
    let (ev_tx, ev_rx) = mpsc::channel::<Event>();
    let fs_tx = ev_tx.clone();
    let thread = std::thread::spawn(move || run(cfg, probe, on_sessions, on_refresh, fs_tx, ev_rx));
    let thread_id = thread.thread().id();
    WatcherHandle {
        events: ev_tx,
        thread: Some(thread),
        thread_id,
    }
}

fn run<F, R>(
    cfg: WatcherConfig,
    probe: Arc<dyn ProcessProbe>,
    on_sessions: F,
    on_refresh: R,
    fs_tx: mpsc::Sender<Event>,
    ev_rx: mpsc::Receiver<Event>,
) where
    F: Fn(Vec<LiveSession>),
    R: Fn(),
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
    let mut watching = try_watch(watcher.as_mut(), &dir, cfg.poll, false);

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
                on_refresh();
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
                    watching = try_watch(watcher.as_mut(), &dir, cfg.poll, true);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Never creates the directory: if it is absent, `watch` fails and we poll instead.
///
/// `is_retry` distinguishes the initial attempt from a later poll-tick retry:
/// the initial attempt logs its failure once (with the poll interval, so a
/// caller knows the cadence to expect); a later retry stays silent on repeated
/// failure (already logged), but logs once, on success, the `false -> true`
/// promotion back onto the filesystem watch.
fn try_watch(
    watcher: Option<&mut notify::RecommendedWatcher>,
    dir: &Path,
    poll: Duration,
    is_retry: bool,
) -> bool {
    let Some(w) = watcher else { return false };
    match w.watch(dir, RecursiveMode::NonRecursive) {
        Ok(()) => {
            if is_retry {
                eprintln!(
                    "perch: watcher: now watching {} (was polling)",
                    dir.display()
                );
            }
            true
        }
        Err(err) => {
            if !is_retry {
                eprintln!(
                    "perch: watcher: failed to watch {}: {err} — polling every {poll:?} and retrying",
                    dir.display()
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
    use std::sync::Mutex;

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

    #[test]
    fn emits_once_on_start_and_again_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        write_record(tmp.path(), 1, "one");
        let (tx, rx) = mpsc::channel();
        // Poll is 30s -- far longer than this test's own deadline below -- so
        // the second emit this test waits for can only have arrived via the
        // filesystem watch + debounce path, not the poll backstop. A
        // poll-only implementation that silently ignored filesystem events
        // could not pass this test.
        let h = spawn(
            WatcherConfig {
                sessions_dir: tmp.path().to_path_buf(),
                debounce: Duration::from_millis(50),
                poll: Duration::from_secs(30),
            },
            Arc::new(AllAlive),
            move |s| {
                let _ = tx.send(s);
            },
            || {},
        );

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
            "a new record must surface via the filesystem watch (poll is 30s, far longer than this test's 3s deadline)"
        );
        h.stop();
    }

    #[test]
    fn missing_directory_falls_back_to_polling_and_never_creates_it() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("sessions");
        let (tx, rx) = mpsc::channel();
        // A 2s poll: long enough that, after promotion, a change surfacing
        // well under that interval can only be explained by the filesystem
        // watch having taken over -- not by "the next poll happened to catch
        // it", which a broken promotion retry could also produce.
        let h = spawn(
            WatcherConfig {
                sessions_dir: missing.clone(),
                debounce: Duration::from_millis(50),
                poll: Duration::from_secs(2),
            },
            Arc::new(AllAlive),
            move |s| {
                let _ = tx.send(s);
            },
            || {},
        );

        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("poll-only still emits");
        assert!(first.is_empty());
        assert!(
            !missing.exists(),
            "watcher must never create the sessions dir"
        );

        // Directory appears later: the retry should pick up the record on
        // the next poll tick.
        std::fs::create_dir_all(&missing).unwrap();
        write_record(&missing, 3, "three");
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut got_first = false;
        while Instant::now() < deadline {
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(200)) {
                if s.len() == 1 {
                    got_first = true;
                    break;
                }
            }
        }
        assert!(
            got_first,
            "record written after the dir appears must surface"
        );

        // Prove promotion actually happened, rather than inferring it from
        // "an update eventually arrived": a further change must now surface
        // well inside the 2s poll interval, which is only possible if the
        // retry above actually switched us onto the filesystem watch.
        //
        // The retry's `try_watch()` call lands just *after* the emit that
        // reported `got_first`, so there is a brief window right at
        // promotion where a single write can race the watch registration
        // and be missed. Rather than papering over that with a blind sleep,
        // keep nudging the file (each write distinct, so each is a real
        // filesystem event) until one is observed — deterministic once the
        // watch is truly live, and still bounded well under the poll.
        let promoted_deadline = Instant::now() + Duration::from_millis(1500);
        let mut promoted = false;
        let mut attempt = 0;
        while Instant::now() < promoted_deadline {
            attempt += 1;
            write_record(&missing, 4, &format!("four-{attempt}"));
            if let Ok(s) = rx.recv_timeout(Duration::from_millis(100)) {
                if s.len() == 2 {
                    promoted = true;
                    break;
                }
            }
        }
        assert!(
            promoted,
            "a change after promotion must surface via the watch, well under the 2s poll interval"
        );
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
            || {},
        );
        rx.recv_timeout(Duration::from_secs(2)).expect("initial");
        h.refresh();
        rx.recv_timeout(Duration::from_secs(1))
            .expect("refresh must not wait for the 30s poll");
        h.stop();
    }

    #[test]
    fn stop_from_inside_the_callback_does_not_deadlock_or_panic() {
        // `on_sessions` runs synchronously on the watcher thread. A caller
        // that stashes the handle somewhere reachable from the callback (the
        // FFI layer plausibly will) and calls `stop()` from inside it must
        // not have that thread join itself.
        //
        // On this platform a self-join does not hang: `pthread_join` on your
        // own thread returns EDEADLK, which `JoinHandle::join()` turns into a
        // panic ("failed to join thread: Resource deadlock avoided"). That
        // panic happens entirely on the detached watcher thread; a test that
        // only sleeps and never observes that thread (as an earlier version
        // of this test did) passes whether or not the guard in `stop()`
        // exists. So this version captures what `stop()` actually does --
        // returns cleanly, or panics -- via `catch_unwind` inside the
        // callback, sends that outcome back over a channel, and asserts on
        // it directly. A bounded `recv_timeout` also covers the case where
        // some other platform's self-join genuinely hangs instead of
        // panicking: this test then fails via timeout rather than hanging
        // the test binary itself.
        let tmp = tempfile::tempdir().unwrap();
        let handle_slot: Arc<Mutex<Option<WatcherHandle>>> = Arc::new(Mutex::new(None));
        let slot = handle_slot.clone();
        let (outcome_tx, outcome_rx) = mpsc::channel::<std::thread::Result<()>>();
        let h = spawn(
            WatcherConfig {
                sessions_dir: tmp.path().to_path_buf(),
                debounce: Duration::from_millis(10),
                poll: Duration::from_millis(50),
            },
            Arc::new(AllAlive),
            move |_s| {
                if let Some(h) = slot.lock().unwrap().take() {
                    let outcome =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.stop()));
                    let _ = outcome_tx.send(outcome);
                }
            },
            || {},
        );
        *handle_slot.lock().unwrap() = Some(h);

        let outcome = outcome_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stop() called from inside its own callback must return within 2s, not hang");
        assert!(
            outcome.is_ok(),
            "stop() called from inside its own callback must not panic (e.g. via a self-join)"
        );
    }
}
