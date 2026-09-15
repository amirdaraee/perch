//! `perch-mcp`: a local MCP server over Perch's index, spoken on stdio.
//!
//! Register it with Claude Code:
//!
//! ```sh
//! claude mcp add perch -- /Applications/Perch.app/Contents/MacOS/perch-mcp
//! ```
//!
//! It opens the index the Perch app maintains read-only, reads Claude Code's
//! live-session records the same way the app does, and never writes anything,
//! anywhere. It has no network code at all.

mod protocol;
mod tools;

use perch_core::config;
use perch_core::db::{self, Db};
use perch_core::live::{self, LiveSession};
use perch_core::platform::RealProcessProbe;
use perch_core::settings::{store, Settings};
use std::path::{Path, PathBuf};

/// The real world: Perch's own data directory, the user's settings, and the
/// system clock — resolved exactly as the app resolves them.
struct RealHost {
    db_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
}

impl RealHost {
    fn new() -> Self {
        Self {
            db_path: store::app_data_dir().ok().map(|d| d.join("index.db")),
            config_path: store::config_path().ok(),
        }
    }
}

impl tools::Host for RealHost {
    fn open_index(&self) -> Result<Db, String> {
        match &self.db_path {
            Some(path) => open_index(path),
            None => Err("could not locate Perch's data directory".into()),
        }
    }

    fn live_sessions(&self) -> Vec<LiveSession> {
        // The settings file's own Claude Code directory wins over the
        // environment, the same precedence the app uses.
        let configured = self.settings().claude_config_dir.trim().to_string();
        let dir = if configured.is_empty() {
            config::config_dir()
        } else {
            Some(PathBuf::from(configured))
        };
        dir.map(|d| live::live_sessions(&config::sessions_dir(&d), &RealProcessProbe))
            .unwrap_or_default()
    }

    fn settings(&self) -> Settings {
        self.config_path
            .as_deref()
            .map(|p| store::load(p).settings)
            .unwrap_or_default()
    }

    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    fn index_updated_at(&self) -> Option<i64> {
        let path = self.db_path.as_ref()?;
        // The app writes through WAL, so the -wal file moves first.
        [
            path.clone(),
            PathBuf::from(format!("{}-wal", path.display())),
        ]
        .iter()
        .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .max()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
    }
}

/// The index read-only, with the one sentence a missing index deserves.
pub(crate) fn open_index(path: &Path) -> Result<Db, String> {
    if !path.is_file() {
        return Err("Perch has not indexed anything yet — open Perch once.".into());
    }
    db::open_read_only(path).map_err(|e| format!("could not open Perch's index: {e}"))
}

fn main() {
    let host = RealHost::new();
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    if let Err(e) = protocol::serve(stdin, stdout, &host) {
        eprintln!("perch-mcp: {e}");
        std::process::exit(1);
    }
}
