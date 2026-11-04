use super::ProcessProbe;

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
    fn process_name(&self, pid: i32) -> Option<String> {
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
