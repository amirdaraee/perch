//! Project discovery. Real paths come from transcript `cwd`, never from slugs.

use crate::transcript::parse_line;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub const WORKTREE_MARKER: &str = "/.claude/worktrees/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredProject {
    pub slug: String,
    pub dir: PathBuf,
    pub real_path: String,
    /// True when `real_path` was guessed from the slug because no session
    /// file carried a `cwd`. The UI must mark these as uncertain.
    pub path_is_guess: bool,
    pub session_files: Vec<PathBuf>,
}

/// Last-resort decoding. Lossy: the encoder flattens both `/` and `.` to `-`,
/// so this can only ever be a guess. Never call it when a `cwd` is available.
pub fn slug_to_guess(slug: &str) -> String {
    slug.replace('-', "/")
}

/// A worktree lives at `<parent>/.claude/worktrees/<name>`.
pub fn parent_path_of(real_path: &str) -> Option<String> {
    real_path
        .find(WORKTREE_MARKER)
        .map(|i| real_path[..i].to_string())
}

/// Read the first `cwd` found in any session file in this directory.
pub fn real_path_for(project_dir: &Path) -> Option<String> {
    for file in session_files_in(project_dir) {
        let Ok(fh) = fs::File::open(&file) else {
            continue;
        };
        // Transcript lines can be very large; cap how many we inspect.
        for line in BufReader::new(fh).lines().take(50) {
            let Ok(line) = line else { break };
            if let Some(parsed) = parse_line(&line) {
                if let Some(cwd) = parsed.cwd {
                    return Some(cwd);
                }
            }
        }
    }
    None
}

fn session_files_in(project_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(project_dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();
    files
}

pub fn discover(projects_root: &Path) -> std::io::Result<Vec<DiscoveredProject>> {
    let entries = match fs::read_dir(projects_root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(slug) = dir.file_name().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };

        let session_files = session_files_in(&dir);
        let (real_path, path_is_guess) = match real_path_for(&dir) {
            Some(cwd) => (cwd, false),
            None => (slug_to_guess(&slug), true),
        };

        out.push(DiscoveredProject {
            slug,
            dir,
            real_path,
            path_is_guess,
            session_files,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_project(root: &Path, slug: &str, cwd: Option<&str>) -> PathBuf {
        let dir = root.join(slug);
        fs::create_dir_all(&dir).unwrap();
        if let Some(cwd) = cwd {
            let f = dir.join("11111111-1111-1111-1111-111111111111.jsonl");
            let mut fh = fs::File::create(f).unwrap();
            // First line is a meta line with no cwd, to prove we keep looking.
            writeln!(fh, r#"{{"type":"last-prompt","leafUuid":"x"}}"#).unwrap();
            writeln!(
                fh,
                r#"{{"type":"user","cwd":"{cwd}","timestamp":"2026-08-18T10:00:00.000Z"}}"#
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn reads_real_path_from_cwd_not_from_slug() {
        let tmp = tempfile::tempdir().unwrap();
        // This slug is genuinely ambiguous: the true path contains a dot.
        let dir = make_project(
            tmp.path(),
            "-Users-a-00-projects-amirdaraee-github-io",
            Some("/Users/a/00/projects/amirdaraee.github.io"),
        );
        assert_eq!(
            real_path_for(&dir).as_deref(),
            Some("/Users/a/00/projects/amirdaraee.github.io")
        );
    }

    #[test]
    fn slug_guess_is_only_a_fallback_and_is_marked() {
        let tmp = tempfile::tempdir().unwrap();
        make_project(tmp.path(), "-Users-a-00-projects-empty-one", None);
        let found = discover(tmp.path()).unwrap();
        let p = found
            .iter()
            .find(|p| p.slug.ends_with("empty-one"))
            .unwrap();
        assert!(
            p.path_is_guess,
            "a project with no readable cwd must be flagged"
        );
        assert_eq!(p.real_path, "/Users/a/00/projects/empty/one");
    }

    #[test]
    fn slug_guess_replaces_leading_and_inner_dashes_with_slashes() {
        assert_eq!(slug_to_guess("-Users-a-proj"), "/Users/a/proj");
        assert_eq!(slug_to_guess("Users-a-proj"), "Users/a/proj");
    }

    #[test]
    fn discovers_projects_and_their_session_files() {
        let tmp = tempfile::tempdir().unwrap();
        make_project(tmp.path(), "-Users-a-one", Some("/Users/a/one"));
        make_project(tmp.path(), "-Users-a-two", Some("/Users/a/two"));

        let mut found = discover(tmp.path()).unwrap();
        found.sort_by(|a, b| a.slug.cmp(&b.slug));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].real_path, "/Users/a/one");
        assert_eq!(found[0].session_files.len(), 1);
        assert!(!found[0].path_is_guess);
    }

    #[test]
    fn ignores_non_directories_and_non_jsonl_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = make_project(tmp.path(), "-Users-a-one", Some("/Users/a/one"));
        fs::write(dir.join("notes.txt"), "hello").unwrap();
        fs::create_dir_all(dir.join("memory")).unwrap();
        fs::write(tmp.path().join("stray.json"), "{}").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_files.len(), 1);
    }

    #[test]
    fn detects_worktree_parent_path() {
        assert_eq!(
            parent_path_of(
                "/Users/a/00/projects/personal-dashboard/.claude/worktrees/finance-section"
            )
            .as_deref(),
            Some("/Users/a/00/projects/personal-dashboard")
        );
        assert_eq!(
            parent_path_of("/Users/a/00/projects/personal-dashboard"),
            None
        );
    }

    #[test]
    fn missing_root_yields_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        let found = discover(&tmp.path().join("does-not-exist")).unwrap();
        assert!(found.is_empty());
    }
}
