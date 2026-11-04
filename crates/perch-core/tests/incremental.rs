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

fn response(id: &str, ts: &str, cwd: &str, output: u64) -> String {
    format!(
        r#"{{"type":"assistant","isSidechain":true,"cwd":"{cwd}","timestamp":"{ts}","message":{{"id":"{id}","model":"claude-fable-5","usage":{{"input_tokens":0,"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
    )
}

fn subagent_path(root: &Path, slug: &str, session: &str, agent: &str) -> std::path::PathBuf {
    root.join(slug)
        .join(session)
        .join("subagents")
        .join(format!("agent-{agent}.jsonl"))
}

fn write_subagent(root: &Path, slug: &str, session: &str, agent: &str, lines: &[String]) {
    let p = subagent_path(root, slug, session, agent);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let mut f = fs::File::create(p).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

const CWD: &str = "/Users/a/one";
const SLUG: &str = "-Users-a-one";

#[test]
fn subagent_turns_count_towards_the_session_that_ran_them() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        SLUG,
        "aaaa",
        &[response("own", "2026-08-18T10:00:00.000Z", CWD, 1)],
    );
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x1",
        &[
            response("sub", "2026-08-18T10:00:01.000Z", CWD, 10),
            response("sub", "2026-08-18T10:00:02.000Z", CWD, 20),
        ],
    );

    let db = open_in_memory().unwrap();
    let stats = index_all(&db, root).unwrap();

    assert_eq!(stats.sessions, 1, "a subagent is not a session of its own");
    assert_eq!(stats.subagents, 1);
    assert_eq!(db.session_count().unwrap(), 1);
    assert_eq!(db.turn_count().unwrap(), 2);
    assert_eq!(db.turn_totals().unwrap().1, 21);
    let session_turns: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE session_id = 'aaaa'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(session_turns, 2);
}

#[test]
fn a_second_pass_adds_nothing_and_new_subagent_lines_are_picked_up() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        SLUG,
        "aaaa",
        &[response("own", "2026-08-18T10:00:00.000Z", CWD, 1)],
    );
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x1",
        &[response("sub", "2026-08-18T10:00:01.000Z", CWD, 10)],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    let again = index_all(&db, root).unwrap();
    assert_eq!(again.new_turns, 0);
    assert_eq!(db.turn_totals().unwrap().1, 11);

    // The same response goes on growing while it streams, plus a new one.
    let mut f = fs::File::options()
        .append(true)
        .open(subagent_path(root, SLUG, "aaaa", "x1"))
        .unwrap();
    writeln!(
        f,
        "{}",
        response("sub", "2026-08-18T10:00:02.000Z", CWD, 30)
    )
    .unwrap();
    writeln!(
        f,
        "{}",
        response("next", "2026-08-18T10:00:03.000Z", CWD, 5)
    )
    .unwrap();
    drop(f);

    index_all(&db, root).unwrap();
    assert_eq!(db.turn_count().unwrap(), 3);
    assert_eq!(db.turn_totals().unwrap().1, 1 + 30 + 5);
}

#[test]
fn a_shrunken_subagent_transcript_replaces_only_its_own_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        SLUG,
        "aaaa",
        &[response("own", "2026-08-18T10:00:00.000Z", CWD, 1)],
    );
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x1",
        &[
            response("old1", "2026-08-18T10:00:01.000Z", CWD, 10),
            response("old2", "2026-08-18T10:00:02.000Z", CWD, 20),
        ],
    );
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x2",
        &[response("other", "2026-08-18T10:00:03.000Z", CWD, 100)],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x1",
        &[response("new", "2026-08-18T10:00:04.000Z", CWD, 7)],
    );
    index_all(&db, root).unwrap();

    assert_eq!(db.turn_totals().unwrap().1, 1 + 7 + 100);
}

#[test]
fn only_agent_transcripts_in_the_subagents_folder_are_read() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        SLUG,
        "aaaa",
        &[response("own", "2026-08-18T10:00:00.000Z", CWD, 1)],
    );
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x1",
        &[response("sub", "2026-08-18T10:00:01.000Z", CWD, 10)],
    );
    let dir = root.join(SLUG).join("aaaa").join("subagents");
    let usage_line = response("stray", "2026-08-18T10:00:02.000Z", CWD, 500);
    fs::write(dir.join("agent-x1.meta.json"), &usage_line).unwrap();
    fs::write(dir.join("notes.jsonl"), format!("{usage_line}\n")).unwrap();

    let db = open_in_memory().unwrap();
    let stats = index_all(&db, root).unwrap();

    assert_eq!(stats.subagents, 1);
    assert_eq!(db.turn_totals().unwrap().1, 11);
}

#[test]
fn a_response_repeated_by_a_resumed_session_is_billed_once() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let shared = response("shared", "2026-08-18T10:00:00.000Z", CWD, 9);
    write_session(root, SLUG, "aaaa", std::slice::from_ref(&shared));
    write_session(
        root,
        SLUG,
        "bbbb",
        &[
            shared,
            response("later", "2026-08-18T10:00:01.000Z", CWD, 1),
        ],
    );

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();

    assert_eq!(db.turn_count().unwrap(), 2);
    assert_eq!(db.turn_totals().unwrap().1, 10);
}

#[test]
fn a_subagents_type_is_read_and_its_description_is_not_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        SLUG,
        "aaaa",
        &[response("own", "2026-08-18T10:00:00.000Z", CWD, 1)],
    );
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x1",
        &[response("sub", "2026-08-18T10:00:01.000Z", CWD, 10)],
    );
    let secret = "sentinel-task-description";
    fs::write(
        root.join(SLUG)
            .join("aaaa")
            .join("subagents")
            .join("agent-x1.meta.json"),
        format!(r#"{{"agentType":"Explore","description":"{secret}"}}"#),
    )
    .unwrap();

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();

    let rows = perch_core::query::session_subagents(&db, "aaaa").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent_type.as_deref(), Some("Explore"));
    assert_eq!(rows[0].usage.output, 10);

    // Nowhere in the index, whichever table you look in.
    let mut stmt = db
        .conn()
        .prepare("SELECT agent_id, agent_type, file_path FROM subagents")
        .unwrap();
    let stored: Vec<String> = stmt
        .query_map([], |r| {
            Ok(format!(
                "{}{}{}",
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, String>(2)?
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!stored.join("").contains(secret));
}

#[test]
fn a_subagent_without_usable_metadata_still_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_session(
        root,
        SLUG,
        "aaaa",
        &[response("own", "2026-08-18T10:00:00.000Z", CWD, 1)],
    );
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x1",
        &[response("sub1", "2026-08-18T10:00:01.000Z", CWD, 10)],
    );
    write_subagent(
        root,
        SLUG,
        "aaaa",
        "x2",
        &[response("sub2", "2026-08-18T10:00:01.000Z", CWD, 20)],
    );
    fs::write(
        root.join(SLUG)
            .join("aaaa")
            .join("subagents")
            .join("agent-x2.meta.json"),
        "{not json",
    )
    .unwrap();

    let db = open_in_memory().unwrap();
    index_all(&db, root).unwrap();

    let rows = perch_core::query::session_subagents(&db, "aaaa").unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.agent_type.is_none()));
}
