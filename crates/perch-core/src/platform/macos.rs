use super::ProcessProbe;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Default, Clone, Copy)]
pub struct RealProcessProbe;

impl ProcessProbe for RealProcessProbe {
    /// `kill(pid, 0)` sends no signal; it only checks the process exists and
    /// is signalable by this user.
    fn is_alive(&self, pid: i32) -> bool {
        if pid <= 0 {
            return false;
        }
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// `comm=` returns just the executable path with no arguments, unlike
    /// `command=` which includes argv and can false-positive on unrelated
    /// processes whose arguments merely contain a matching substring (e.g.
    /// `vim ~/projects/claude-dashboard/x.rs`).
    ///
    /// Remembered per process, because this runs for every session on every
    /// tick and starting a `ps` each time was most of what a tick cost in other
    /// processes. The kernel's own `proc_name` would avoid `ps` altogether, but
    /// it doesn't report Claude Code as `claude`. The start time is what stops a
    /// recycled pid from inheriting the name of the process that had it before.
    fn process_name(&self, pid: i32) -> Option<String> {
        let Some(started) = start_time(pid) else {
            return name_from_ps(pid);
        };
        let mut known = KNOWN_NAMES.lock().unwrap_or_else(|e| e.into_inner());
        let names = known.get_or_insert_with(HashMap::new);
        if let Some((when, name)) = names.get(&pid) {
            if *when == started {
                return Some(name.clone());
            }
        }
        let name = name_from_ps(pid)?;
        names.insert(pid, (started, name.clone()));
        Some(name)
    }
}

/// A process's start time, in seconds and microseconds, and its name.
type KnownName = ((u64, u64), String);

static KNOWN_NAMES: Mutex<Option<HashMap<i32, KnownName>>> = Mutex::new(None);

/// When this process started, or `None` if it's gone or can't be examined.
fn start_time(pid: i32) -> Option<(u64, u64)> {
    if pid <= 0 {
        return None;
    }
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let got = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (got == size).then_some((info.pbi_start_tvsec, info.pbi_start_tvusec))
}

fn name_from_ps(pid: i32) -> Option<String> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        return None;
    }
    std::path::Path::new(&name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// Every process as `pid ppid %cpu rss-in-kb`, for `crate::process`. Numbers only,
/// never a command line. `%cpu` is `ps`'s own decaying average, so one read means
/// something without Perch having to sample over time. `LC_ALL=C` keeps the
/// decimal separator a full stop under every language setting.
pub fn process_table() -> Option<String> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pcpu=,rss="])
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_real_process_every_time_and_forgets_it_when_it_ends() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        // Time for the exec to finish, so the name isn't caught mid-change.
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert_eq!(RealProcessProbe.process_name(pid).as_deref(), Some("sleep"));
        assert_eq!(RealProcessProbe.process_name(pid).as_deref(), Some("sleep"));

        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(RealProcessProbe.process_name(pid), None);
    }

    #[test]
    fn a_different_start_time_means_a_different_process() {
        let mut names = KNOWN_NAMES.lock().unwrap_or_else(|e| e.into_inner());
        let me = std::process::id() as i32;
        names
            .get_or_insert_with(HashMap::new)
            .insert(me, ((1, 1), "not-me".to_string()));
        drop(names);

        assert_ne!(RealProcessProbe.process_name(me).as_deref(), Some("not-me"));
    }

    #[test]
    fn nothing_is_known_about_an_impossible_pid() {
        assert_eq!(start_time(0), None);
        assert_eq!(start_time(-1), None);
    }
}
