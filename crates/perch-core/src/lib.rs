//! Perch core: reads Claude Code's on-disk data. Never writes to it.

pub mod actions;
pub mod config;
pub mod db;
pub mod discovery;
pub mod index;
pub mod live;
pub mod model;
pub mod platform;
pub mod pricing;
pub mod query;
pub mod scan;
pub mod settings;
pub mod transcript;
pub mod ui;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
