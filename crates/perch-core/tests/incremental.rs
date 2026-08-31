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
