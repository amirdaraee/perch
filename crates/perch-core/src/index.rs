//! Composes discovery, scanning, and persistence into one indexing pass.

use crate::db::Db;
use crate::discovery::{discover, parent_path_of};
use crate::model::SessionRecord;
use crate::scan::scan_from;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct IndexStats {
    pub projects: usize,
    pub sessions: usize,
    pub new_turns: usize,
    pub bytes_read: u64,
    pub lines_skipped: u64,
    /// Sessions the pass could not read at all — a stat failure, a permission
    /// error, or a file that vanished mid-pass. Such a session contributes to no
    /// total, so without this counter it would disappear from the report with no
    /// signal anywhere that anything was missed.
    pub sessions_skipped: usize,
}

pub fn index_all(db: &Db, projects_root: &Path) -> Result<IndexStats> {
    let projects = discover(projects_root)?;
    let mut stats = IndexStats {
        projects: projects.len(),
        ..Default::default()
    };

    // Pass 1: projects, sessions, turns.
    let mut ids_by_path: HashMap<String, i64> = HashMap::new();
    let mut worktrees: Vec<(i64, String)> = Vec::new();

    for project in &projects {
        let project_id =
            db.upsert_project(&project.slug, &project.real_path, project.path_is_guess)?;
        ids_by_path.insert(project.real_path.clone(), project_id);

        if let Some(parent) = parent_path_of(&project.real_path) {
            worktrees.push((project_id, parent));
        }

        for file in &project.session_files {
            let Some(session_id) = file.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let offset = db.session_offset(session_id)?;
            let file_size = match std::fs::metadata(file) {
                Ok(m) => m.len(),
                // Cannot stat the file: skip this session entirely and leave any
                // previously indexed data untouched. A stat failure must never be
                // mistaken for a shrink, which would delete stored turns.
                Err(_) => {
                    stats.sessions_skipped += 1;
                    continue;
                }
            };

            let outcome = match scan_from(file, offset) {
                Ok(o) => o,
                // Unreadable file: skip, never fatal.
                Err(_) => {
                    stats.sessions_skipped += 1;
                    continue;
                }
            };

            // `scan_from` restarts at 0 when the file shrank, and reports it.
            // The decision is made once, inside the scan, from the same `stat`
            // the scan itself read: deciding it here from a second `stat` could
            // disagree with the scan if the file changed in between, and would
            // then either lose turns or re-ingest the whole file on top of the
            // rows already stored.
            let restarted = outcome.restarted;

            stats.sessions += 1;
            stats.new_turns += outcome.turns.len();
            stats.bytes_read +=
                outcome
                    .new_offset
                    .saturating_sub(if restarted { 0 } else { offset });
            stats.lines_skipped += outcome.lines_skipped;

            let existing_messages = if offset == 0 || restarted {
                0
            } else {
                db.session_message_count(session_id)?
            };

            // One transaction: dropping stale turns, advancing the offset, and
            // storing the new turns must all happen or none of them must.
            db.apply_scan(
                &SessionRecord {
                    id: session_id.to_string(),
                    project_id,
                    file_path: file.to_string_lossy().into_owned(),
                    file_size,
                    indexed_offset: outcome.new_offset,
                    started_at: outcome.meta.first_ts,
                    last_activity_at: outcome.meta.last_ts,
                    cwd: outcome.meta.cwd.clone(),
                    git_branch: outcome.meta.git_branch.clone(),
                    cc_version: outcome.meta.cc_version.clone(),
                    title: outcome.meta.ai_title.clone(),
                    message_count: existing_messages + outcome.meta.message_count,
                },
                &outcome.turns,
                restarted,
            )?;
        }
    }

    // Pass 2: link worktrees, now that every project has an id.
    for (child_id, parent_path) in worktrees {
        let parent_id = ids_by_path.get(&parent_path).copied();
        db.set_parent(child_id, parent_id)?;
    }

    Ok(stats)
}
