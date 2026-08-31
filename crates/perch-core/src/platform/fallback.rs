use super::ProcessProbe;

/// Non-macOS builds exist so `perch-core` keeps compiling and testing on Linux.
/// These are honest stubs, not silent lies: they report nothing is alive, so a
/// non-macOS build shows no live sessions rather than wrong ones.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealProcessProbe;

impl ProcessProbe for RealProcessProbe {
    fn is_alive(&self, _pid: i32) -> bool {
        false
    }
    fn cmdline_contains(&self, _pid: i32, _needle: &str) -> bool {
        false
    }
}
