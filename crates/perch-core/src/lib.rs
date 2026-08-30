//! Perch core: reads Claude Code's on-disk data. Never writes to it.

pub mod config;
pub mod model;
pub mod scan;
pub mod transcript;

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_builds() {
        assert_eq!(2 + 2, 4);
    }
}
