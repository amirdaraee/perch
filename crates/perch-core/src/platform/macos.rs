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

    fn cmdline_contains(&self, pid: i32, needle: &str) -> bool {
        let Ok(out) = std::process::Command::new("/bin/ps")
            .args(["-o", "command=", "-p", &pid.to_string()])
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout).contains(needle)
    }
}
