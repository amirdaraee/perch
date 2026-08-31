use perch_core::db::open_in_memory;
use perch_core::index::index_all;
use std::fs;
use std::io::Write;
use std::path::Path;

fn assistant(ts: &str, cwd: &str, input: u64, output: u64) -> String {
    format!(
        r#"{{"type":"assistant","cwd":"{cwd}","gitBranch":"main","version":"2.1.1","timestamp":"{ts}","message":{{"model":"claude-fable-5","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
    )
}

fn write_session(root: &Path, slug: &str, id: &str, lines: &[String]) {
    let dir = root.join(slug);
    fs::create_dir_all(&dir).unwrap();
    let mut f = fs::File::create(dir.join(format!("{id}.jsonl"))).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

fn append_session(root: &Path, slug: &str, id: &str, lines: &[String]) {
    let p = root.join(slug).join(format!("{id}.jsonl"));
    let mut f = fs::File::options().append(true).open(p).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

#[test]
fn indexes_projects_sessions_and_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[
            assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2),
            assistant("2026-08-18T10:01:00.000Z", "/Users/a/one", 3, 4),
        ],
    );

    let db = open_in_memory().unwrap();
    let stats = index_all(&db, root).unwrap();

    assert_eq!(stats.projects, 1);
    assert_eq!(stats.sessions, 1);
    assert_eq!(stats.new_turns, 2);
    assert_eq!(db.turn_count().unwrap(), 2);
}

/// The property the whole performance strategy rests on.
#[test]
fn reindexing_after_append_adds_only_new_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2)],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    assert_eq!(db.turn_count().unwrap(), 1);

    append_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[assistant("2026-08-18T10:01:00.000Z", "/Users/a/one", 3, 4)],
    );

    let stats = index_all(&db, root).unwrap();
    assert_eq!(stats.new_turns, 1, "must index only the appended turn");
    assert_eq!(
        db.turn_count().unwrap(),
        2,
        "must not duplicate the first turn"
    );
}

#[test]
fn reindexing_unchanged_data_is_a_no_op() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2)],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    let stats = index_all(&db, root).unwrap();

    assert_eq!(stats.new_turns, 0);
    assert_eq!(stats.bytes_read, 0);
    assert_eq!(db.turn_count().unwrap(), 1);
}

/// A pass that reads no new bytes yields an all-`None` `SessionMeta`. Those
/// `None`s must never be written over the values an earlier pass stored —
/// `cwd` in particular is the only source of a project's real path.
#[test]
fn reindexing_unchanged_data_preserves_session_metadata() {
    /// The five indexer-owned session columns, all cast to text so one helper
    /// can read them together.
    fn columns(db: &perch_core::db::Db) -> Vec<Option<String>> {
        db.conn()
            .query_row(
                "SELECT cwd, git_branch, cc_version,
                        CAST(last_activity_at AS TEXT), CAST(started_at AS TEXT)
                 FROM sessions WHERE id = 'aaaa'",
                [],
                |r| Ok(vec![r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?]),
            )
            .unwrap()
    }

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2)],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();

    let first = columns(&db);
    assert_eq!(first[0].as_deref(), Some("/Users/a/one"), "cwd on pass 1");
    assert_eq!(first[1].as_deref(), Some("main"), "git_branch on pass 1");
    assert_eq!(first[2].as_deref(), Some("2.1.1"), "cc_version on pass 1");
    assert!(first[3].is_some(), "last_activity_at on pass 1");
    assert!(first[4].is_some(), "started_at on pass 1");

    // A second pass over unchanged bytes: `scan_from` returns an empty
    // `SessionMeta`, and nothing in it may reach the stored row.
    index_all(&db, root).unwrap();

    let second = columns(&db);
    for (i, name) in [
        "cwd",
        "git_branch",
        "cc_version",
        "last_activity_at",
        "started_at",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(second[i], first[i], "{name} must survive a re-index");
    }
}

#[test]
fn links_worktrees_to_their_parent_project() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-proj",
        "aaaa",
        &[assistant("2026-08-18T10:00:00.000Z", "/Users/a/proj", 1, 2)],
    );
    write_session(
        root,
        "-Users-a-proj--claude-worktrees-feature",
        "bbbb",
        &[assistant(
            "2026-08-18T10:00:00.000Z",
            "/Users/a/proj/.claude/worktrees/feature",
            1,
            2,
        )],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();

    let parent_id: i64 = db
        .conn()
        .query_row(
            "SELECT id FROM projects WHERE real_path = '/Users/a/proj'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let child_parent: Option<i64> = db
        .conn()
        .query_row(
            "SELECT parent_project_id FROM projects WHERE real_path LIKE '%worktrees/feature'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(child_parent, Some(parent_id));
}

#[test]
fn a_truncated_transcript_is_rescanned_without_duplicating_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[
            assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2),
            assistant("2026-08-18T10:01:00.000Z", "/Users/a/one", 3, 4),
            assistant("2026-08-18T10:02:00.000Z", "/Users/a/one", 5, 6),
        ],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    assert_eq!(db.turn_count().unwrap(), 3);

    // The file is replaced by a shorter one — the stored offset is now meaningless.
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[assistant("2026-08-18T11:00:00.000Z", "/Users/a/one", 9, 9)],
    );

    index_all(&db, root).unwrap();
    assert_eq!(
        db.turn_count().unwrap(),
        1,
        "stale turns must be dropped, not duplicated"
    );

    let message_count: i64 = db
        .conn()
        .query_row(
            "SELECT message_count FROM sessions WHERE id = 'aaaa'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        message_count, 1,
        "message count must not accumulate across a rescan"
    );
}

#[test]
fn user_notes_survive_a_reindex() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2)],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    let id: i64 = db
        .conn()
        .query_row("SELECT id FROM projects LIMIT 1", [], |r| r.get(0))
        .unwrap();
    db.set_note(id, "where I left off").unwrap();

    index_all(&db, root).unwrap();
    assert_eq!(db.note(id).unwrap().as_deref(), Some("where I left off"));
}

#[test]
fn a_vanished_transcript_does_not_delete_its_indexed_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[
            assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2),
            assistant("2026-08-18T10:01:00.000Z", "/Users/a/one", 3, 4),
        ],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    assert_eq!(db.turn_count().unwrap(), 2);

    fs::remove_file(root.join("-Users-a-one").join("aaaa.jsonl")).unwrap();

    let stats = index_all(&db, root).unwrap();
    assert_eq!(
        db.turn_count().unwrap(),
        2,
        "a session that can no longer be read must not lose its already-indexed turns"
    );
    // A file that is gone by discovery time is filtered out by `is_file()` and
    // never visited, so it is neither indexed nor skipped. Only a file that
    // survives discovery and then fails to stat or read counts as skipped.
    assert_eq!(stats.sessions, 0);
    assert_eq!(stats.sessions_skipped, 0);
    let session_count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM sessions WHERE id = 'aaaa'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(session_count, 1, "the session row itself must survive too");
}

#[test]
fn one_unreadable_session_does_not_stop_other_projects() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        "-Users-a-one",
        "aaaa",
        &[assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 1, 2)],
    );
    write_session(
        root,
        "-Users-a-two",
        "bbbb",
        &[assistant("2026-08-18T10:00:00.000Z", "/Users/a/two", 3, 4)],
    );
    // A third session file that exists but cannot be opened for reading.
    write_session(
        root,
        "-Users-a-one",
        "cccc",
        &[assistant("2026-08-18T10:00:00.000Z", "/Users/a/one", 5, 6)],
    );
    {
        let unreadable = root.join("-Users-a-one").join("cccc.jsonl");
        let mut perms = fs::metadata(&unreadable).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
        fs::set_permissions(&unreadable, perms).unwrap();
    }

    let db = open_in_memory().unwrap();
    let result = index_all(&db, root);

    // Restore permissions so the tempdir can be cleaned up.
    {
        let unreadable = root.join("-Users-a-one").join("cccc.jsonl");
        let mut perms = fs::metadata(&unreadable).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o644);
        fs::set_permissions(&unreadable, perms).unwrap();
    }

    let stats = result.unwrap();
    assert_eq!(
        db.turn_count().unwrap(),
        2,
        "the other two projects' sessions must still be indexed"
    );
    assert_eq!(
        stats.sessions, 2,
        "only the two readable sessions count as indexed"
    );
    assert_eq!(
        stats.sessions_skipped, 1,
        "a session that could not be read must be counted, not silently dropped"
    );
}
