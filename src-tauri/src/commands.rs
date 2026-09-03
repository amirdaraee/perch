//! Tauri commands over `perch-core`. Read-only with respect to the Claude Code
//! config directory: only Perch's own app-data directory (`db_path`) is ever
//! written. No transcript or message text crosses this boundary — only
//! counts, totals, paths, session names, statuses, and model names.

use perch_core::{config, db, live, platform::RealProcessProbe, pricing, query};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSlice {
    pub tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub window: UsageSlice,
    pub week: UsageSlice,
    pub today: UsageSlice,
    /// "estimated" until the tier-1 usage endpoint lands (spec §8).
    pub source: String,
    /// False when the index holds no turns at all — a freshly created database
    /// that has never been filled. The zeros in that case mean "nothing indexed
    /// yet", not "you spent nothing", and the UI must not present them as
    /// numbers.
    pub has_data: bool,
}

fn config_dir() -> Result<std::path::PathBuf, String> {
    config::config_dir()
        .ok_or_else(|| "could not locate a Claude Code config directory".to_string())
}

/// Perch's own database — never inside the Claude Code directory.
fn db_path() -> Result<std::path::PathBuf, String> {
    let base = macos_app_data_dir()?;
    Ok(base.join("index.db"))
}

/// Hand-rolled macOS `~/Library/Application Support/Perch` path. Not a
/// cross-platform abstraction — a later milestone owns platform paths
/// properly (e.g. via the `dirs`/`directories` crate); this is deliberately
/// named and gated so it is not mistaken for one.
#[cfg(target_os = "macos")]
fn macos_app_data_dir() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    Ok(std::path::PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Perch"))
}

#[cfg(not(target_os = "macos"))]
fn macos_app_data_dir() -> Result<std::path::PathBuf, String> {
    Err("Perch's app-data directory is only defined on macOS".to_string())
}

#[tauri::command]
pub fn live_sessions() -> Result<Vec<live::LiveSession>, String> {
    let cfg = config_dir()?;
    Ok(live::live_sessions(
        &config::sessions_dir(&cfg),
        &RealProcessProbe,
    ))
}

#[tauri::command]
pub fn usage_summary() -> Result<UsageSummary, String> {
    let database = db::open(&db_path()?).map_err(|e| e.to_string())?;
    pricing::seed_default_prices(&database).map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().timestamp_millis();
    let slice = |since: i64| -> Result<UsageSlice, String> {
        let (u, cost) = query::usage_since(&database, since).map_err(|e| e.to_string())?;
        Ok(UsageSlice {
            tokens: u.total_tokens(),
            cost_usd: cost,
        })
    };

    Ok(UsageSummary {
        window: slice(now - 5 * 60 * 60 * 1000)?,
        week: slice(now - 7 * 24 * 60 * 60 * 1000)?,
        today: slice(now - 24 * 60 * 60 * 1000)?,
        source: "estimated".to_string(),
        has_data: database.turn_count().map_err(|e| e.to_string())? > 0,
    })
}

#[tauri::command]
pub fn reindex() -> Result<perch_core::index::IndexStats, String> {
    let cfg = config_dir()?;
    let database = db::open(&db_path()?).map_err(|e| e.to_string())?;
    pricing::seed_default_prices(&database).map_err(|e| e.to_string())?;
    perch_core::index::index_all(&database, &config::projects_dir(&cfg)).map_err(|e| e.to_string())
}
