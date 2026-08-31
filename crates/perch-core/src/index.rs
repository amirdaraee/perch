//! Composes discovery, scanning, and persistence into one indexing pass.

use crate::db::Db;
use crate::discovery::{discover, parent_path_of};
use crate::model::SessionRecord;
use crate::scan::scan_from;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub projects: usize,
    pub sessions: usize,
    pub new_turns: usize,
    pub bytes_read: u64,
    pub lines_skipped: u64,
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
            let file_size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);

            // `scan_from` restarts at 0 when the file shrank. The already-stored
            // turns for this session are then stale duplicates and must be dropped,
            // and the message count must not accumulate on top of them.
            let restarted = file_size < offset;
            if restarted {
                db.delete_turns_for_session(session_id)?;
            }

            let outcome = match scan_from(file, offset) {
                Ok(o) => o,
                Err(_) => continue, // unreadable file: skip, never fatal
            };

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

            db.upsert_session(&SessionRecord {
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
                message_count: existing_messages + outcome.meta.message_count,
            })?;

            db.insert_turns(session_id, &outcome.turns)?;
        }
    }

    // Pass 2: link worktrees, now that every project has an id.
    for (child_id, parent_path) in worktrees {
        let parent_id = ids_by_path.get(&parent_path).copied();
        db.set_parent(child_id, parent_id)?;
    }

    Ok(stats)
}
