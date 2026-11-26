//! Perch core: reads AI coding tools' on-disk data, Claude Code's first.
//! Never writes to it.

pub mod actions;
pub mod codex;
pub mod config;
pub mod days;
pub mod db;
pub mod discovery;
pub mod grok;
pub mod index;
pub mod live;
pub mod model;
pub mod notify;
pub mod opencode;
pub mod platform;
pub mod pricing;
pub mod process;
pub mod provider;
pub mod query;
pub mod readme;
pub mod scan;
pub mod settings;
pub mod snapshot;
pub mod terminals;
pub mod transcript;
pub mod ui;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
