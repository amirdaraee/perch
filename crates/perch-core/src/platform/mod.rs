//! The only OS-bound seams in the data layer. Everything else is portable.

/// Confirms a process is alive AND is the process we think it is.
/// Both are required: pids are recycled, so liveness alone would let a stale
/// session record resurrect as a false "running session" (spec §7).
pub trait ProcessProbe: Send + Sync {
    fn is_alive(&self, pid: i32) -> bool;
    /// The process's executable name (basename of `comm`), if it can be determined.
    fn process_name(&self, pid: i32) -> Option<String>;
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::RealProcessProbe;

#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(not(target_os = "macos"))]
pub use fallback::RealProcessProbe;
