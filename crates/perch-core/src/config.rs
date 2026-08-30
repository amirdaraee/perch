//! Resolution of the Claude Code configuration directory.

use std::path::{Path, PathBuf};

fn non_empty(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.trim().is_empty())
}

/// Pure resolution so precedence is testable without touching the process environment.
pub fn resolve_config_dir(
    claude_config_dir: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(d) = non_empty(claude_config_dir) {
        return Some(PathBuf::from(d));
    }
    if let Some(d) = non_empty(xdg_config_home) {
        return Some(Path::new(d).join("claude"));
    }
    non_empty(home).map(|h| Path::new(h).join(".claude"))
}

/// Reads the real process environment.
pub fn config_dir() -> Option<PathBuf> {
    let claude = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    resolve_config_dir(claude.as_deref(), xdg.as_deref(), home.as_deref())
}

pub fn projects_dir(config: &Path) -> PathBuf {
    config.join("projects")
}

pub fn sessions_dir(config: &Path) -> PathBuf {
    config.join("sessions")
}

pub fn history_file(config: &Path) -> PathBuf {
    config.join("history.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_claude_config_dir() {
        let got = resolve_config_dir(Some("/custom/claude"), Some("/xdg"), Some("/home/u"));
        assert_eq!(got, Some(PathBuf::from("/custom/claude")));
    }

    #[test]
    fn falls_back_to_xdg_config_home() {
        let got = resolve_config_dir(None, Some("/xdg"), Some("/home/u"));
        assert_eq!(got, Some(PathBuf::from("/xdg/claude")));
    }

    #[test]
    fn falls_back_to_home_dot_claude() {
        let got = resolve_config_dir(None, None, Some("/home/u"));
        assert_eq!(got, Some(PathBuf::from("/home/u/.claude")));
    }

    #[test]
    fn returns_none_when_nothing_is_set() {
        assert_eq!(resolve_config_dir(None, None, None), None);
    }

    #[test]
    fn ignores_empty_env_values() {
        let got = resolve_config_dir(Some(""), Some(""), Some("/home/u"));
        assert_eq!(got, Some(PathBuf::from("/home/u/.claude")));
    }

    #[test]
    fn derives_subpaths() {
        let c = Path::new("/home/u/.claude");
        assert_eq!(projects_dir(c), PathBuf::from("/home/u/.claude/projects"));
        assert_eq!(sessions_dir(c), PathBuf::from("/home/u/.claude/sessions"));
        assert_eq!(
            history_file(c),
            PathBuf::from("/home/u/.claude/history.jsonl")
        );
    }
}
