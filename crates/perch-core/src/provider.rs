//! A provider is one AI coding tool whose local records Perch reads. Claude
//! Code is the first; each other tool implements the same trait, so the index,
//! the live list and the launcher never need to know which tool they're
//! talking to.
//!
//! Every provider follows the same rules as Claude Code always has:
//! - **Read-only.** It reads the tool's files and never writes, moves or
//!   deletes them. CI checks this for the whole crate.
//! - **Named paths only.** It reads the files it names, never a recursive walk
//!   of the tool's home, so a credentials file beside the sessions is never
//!   opened.
//! - **No message contents.** Only usage, timing and identifiers reach the
//!   index.
//! - **Unknown stays unknown.** A capability the tool lacks is declared absent
//!   in [`Capabilities`], never guessed or shown as zero.

use crate::actions::TerminalCommand;
use crate::db::Db;
use crate::index::IndexStats;
use crate::live::LiveSession;
use crate::platform::ProcessProbe;
use anyhow::Result;
use std::path::PathBuf;

/// The id of Claude Code's provider, and the provider of every session indexed
/// before there were others. Claude Code's session ids are stored bare.
pub const CLAUDE_CODE: &str = "claude-code";

/// For `serde(default)`, which needs a function.
pub fn claude_code() -> String {
    CLAUDE_CODE.to_string()
}

/// A session id that can't collide with another tool's: Claude Code's ids
/// as they are, anyone else's prefixed with their provider id.
pub fn session_key(provider: &str, raw_id: &str) -> String {
    if provider == CLAUDE_CODE {
        raw_id.to_string()
    } else {
        format!("{provider}:{raw_id}")
    }
}

/// What a tool can tell Perch. The UI leaves out whatever is `false` rather
/// than showing a placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Capabilities {
    /// Which sessions are running right now.
    pub live_sessions: bool,
    /// Whether a running session is waiting on the person.
    pub waiting: bool,
    /// Token counts per response.
    pub tokens: bool,
    /// A command that reopens a past session.
    pub resume: bool,
}

pub trait Provider: Send + Sync {
    /// Stable, lower-case, stored in the index: `"claude-code"`.
    fn id(&self) -> &'static str;
    /// For people: "Claude Code".
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;
    /// The directories this tool's records live in, for a file watcher.
    fn watch_paths(&self) -> Vec<PathBuf>;
    /// Bring the index up to date with this tool's records. Incremental:
    /// a session already read is only read from where it left off, and a
    /// session whose records are gone is kept, since the index may be the
    /// only record left.
    fn index(&self, db: &Db) -> Result<IndexStats>;
    /// The tool's sessions running right now, or none if it can't say.
    fn live_sessions(&self, probe: &dyn ProcessProbe) -> Vec<LiveSession>;
    /// The command that reopens `session_id` in `cwd`, if the tool has one.
    fn resume_command(
        &self,
        session_id: &str,
        cwd: &str,
        bypass_permissions: bool,
    ) -> Option<TerminalCommand>;
}

/// Claude Code, reading the config directory it was started with
/// (`~/.claude`, or `CLAUDE_CONFIG_DIR`).
pub struct ClaudeCode {
    config_dir: PathBuf,
}

impl ClaudeCode {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }
}

impl Provider for ClaudeCode {
    fn id(&self) -> &'static str {
        CLAUDE_CODE
    }

    fn name(&self) -> &'static str {
        "Claude Code"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            live_sessions: true,
            waiting: true,
            tokens: true,
            resume: true,
        }
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![
            crate::config::projects_dir(&self.config_dir),
            crate::config::sessions_dir(&self.config_dir),
        ]
    }

    fn index(&self, db: &Db) -> Result<IndexStats> {
        crate::index::index_all(db, &crate::config::projects_dir(&self.config_dir))
    }

    fn live_sessions(&self, probe: &dyn ProcessProbe) -> Vec<LiveSession> {
        crate::live::live_sessions(&crate::config::sessions_dir(&self.config_dir), probe)
    }

    fn resume_command(
        &self,
        session_id: &str,
        cwd: &str,
        bypass_permissions: bool,
    ) -> Option<TerminalCommand> {
        Some(TerminalCommand::resume(session_id, cwd, bypass_permissions))
    }
}

/// Index every provider, adding up what each pass did. One tool failing
/// doesn't stop the others; the first failure is returned once all have run.
pub fn index_all(db: &Db, providers: &[&dyn Provider]) -> Result<IndexStats> {
    let mut total = IndexStats::default();
    let mut first_error = None;
    for p in providers {
        match p.index(db) {
            Ok(s) => {
                total.projects += s.projects;
                total.sessions += s.sessions;
                total.new_turns += s.new_turns;
                total.bytes_read += s.bytes_read;
                total.lines_skipped += s.lines_skipped;
                total.sessions_skipped += s.sessions_skipped;
                total.subagents += s.subagents;
                total.subagents_skipped += s.subagents_skipped;
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e.context(format!("{} could not be read", p.name())));
                }
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(total),
    }
}

/// Every provider's running sessions, in provider order.
pub fn live_sessions(providers: &[&dyn Provider], probe: &dyn ProcessProbe) -> Vec<LiveSession> {
    providers
        .iter()
        .filter(|p| p.capabilities().live_sessions)
        .flat_map(|p| p.live_sessions(probe))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_ids_stay_bare_and_others_are_namespaced() {
        assert_eq!(session_key(CLAUDE_CODE, "abc"), "abc");
        assert_eq!(session_key("codex", "abc"), "codex:abc");
    }
}
