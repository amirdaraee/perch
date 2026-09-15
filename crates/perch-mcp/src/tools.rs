//! The four tools. Each reads Perch's index through a read-only connection and
//! returns machine data — integer tokens, USD floats, RFC 3339 timestamps — for
//! a model to reason over, not a menu to draw. No transcript contents: session
//! titles are the most a session says about itself here.

use perch_core::db::Db;
use perch_core::live::{LiveSession, SessionStatus};
use perch_core::model::TurnUsage;
use perch_core::settings::Settings;
use perch_core::ui::main_window::{project_group, ProjectGroup};
use perch_core::{pricing, query, readme};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

/// Everything a tool needs from the world, behind a trait so tests can hand it
/// a fixture index, a fixed clock and made-up live sessions.
pub trait Host {
    /// The index, opened read-only, or the sentence explaining why not.
    fn open_index(&self) -> Result<Db, String>;
    fn live_sessions(&self) -> Vec<LiveSession>;
    fn settings(&self) -> Settings;
    fn now_ms(&self) -> i64;
    /// When the app last changed the index, so a caller can tell stale data.
    fn index_updated_at(&self) -> Option<i64>;
}

type ToolResult = Result<Value, String>;

pub fn definitions() -> Value {
    json!([
        {
            "name": "list_projects",
            "title": "List projects",
            "description": "Every Claude Code project Perch knows: name, folder, description from its README, group (pinned/active/recent/archived), session count, lifetime tokens and estimated cost, latest git branch, last activity, and how many sessions are running or waiting on the user right now.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "get_project",
            "title": "Get one project",
            "description": "One project in full, by id (from list_projects) or by folder path: its note, pinned/archived state, daily tokens over the configured chart window, and every session with title, branch, tokens, cost and whether it is live.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Project id from list_projects." },
                    "path": { "type": "string", "description": "The project's absolute folder path." }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "live_sessions",
            "title": "Live sessions",
            "description": "Claude Code sessions running on this Mac right now: status (working, idle, waiting, background, ended), since when, why a waiting session is waiting, and its title and project.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "usage_summary",
            "title": "Usage summary",
            "description": "Token usage and estimated cost over the last N days (default 7, at most 90): totals by token class, by model, the top projects, and a per-day series.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "days": { "type": "integer", "minimum": 1, "maximum": 90, "default": 7 }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

/// `None` for a tool that does not exist; otherwise the tool's own outcome.
pub fn call(host: &dyn Host, name: &str, arguments: &Value) -> Option<ToolResult> {
    let outcome = match name {
        "list_projects" => list_projects(host),
        "get_project" => get_project(host, arguments),
        "live_sessions" => live_sessions(host),
        "usage_summary" => usage_summary(host, arguments),
        _ => return None,
    };
    Some(outcome.map(|mut value| {
        value["index_updated_at"] = iso(host.index_updated_at());
        value
    }))
}

fn list_projects(host: &dyn Host) -> ToolResult {
    let db = host.open_index()?;
    let settings = host.settings();
    let now = host.now_ms();
    let live = host.live_sessions();
    let summaries = query::project_summaries(&db).map_err(read_error)?;
    let branches = query::latest_branches(&db).map_err(read_error)?;

    let mut rows: Vec<(u8, Option<i64>, String, Value)> = summaries
        .into_iter()
        .map(|s| {
            let meta = db.project_meta(s.id).ok();
            let archived = meta.as_ref().is_some_and(|m| m.archived);
            let pinned = meta.as_ref().is_some_and(|m| m.pinned);
            let group = project_group(
                archived,
                pinned,
                s.last_activity_at,
                now,
                settings.active_within_days,
            );
            let name = meta
                .and_then(|m| m.display_name)
                .unwrap_or_else(|| dir_name(&s.real_path));
            let (running, waiting) = live_counts(&live, &s.real_path);
            let value = json!({
                "id": s.id,
                "name": name,
                "path": s.real_path,
                "group": group_name(group),
                "description": readme::project_description(Path::new(&s.real_path)),
                "sessions": s.sessions,
                "tokens": s.usage.total_tokens(),
                "cost_usd": usd(s.cost_usd),
                "branch": branches.get(&s.id),
                "last_active_at": iso(s.last_activity_at),
                "running": running,
                "waiting": waiting,
            });
            (group_rank(group), s.last_activity_at, name, value)
        })
        .collect();
    // The sidebar's order: group, then most recent first, then name.
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.unwrap_or(i64::MIN).cmp(&a.1.unwrap_or(i64::MIN)))
            .then_with(|| a.2.cmp(&b.2))
    });
    Ok(json!({
        "projects": rows.into_iter().map(|r| r.3).collect::<Vec<_>>(),
        "unpriced_models": unpriced_models(&db)?,
    }))
}

fn get_project(host: &dyn Host, arguments: &Value) -> ToolResult {
    let db = host.open_index()?;
    let settings = host.settings();
    let now = host.now_ms();
    let live = host.live_sessions();
    let summaries = query::project_summaries(&db).map_err(read_error)?;

    let summary = match (arguments.get("id"), arguments.get("path")) {
        (Some(id), _) => {
            let id = id.as_i64().ok_or("id must be an integer")?;
            summaries.into_iter().find(|s| s.id == id)
        }
        (None, Some(path)) => {
            let path = path.as_str().ok_or("path must be a string")?;
            let wanted = path.trim_end_matches('/');
            summaries.into_iter().find(|s| s.real_path == wanted)
        }
        (None, None) => return Err("give either id or path".into()),
    }
    .ok_or("no such project in Perch's index")?;

    let meta = db.project_meta(summary.id).map_err(read_error)?;
    let group = project_group(
        meta.archived,
        meta.pinned,
        summary.last_activity_at,
        now,
        settings.active_within_days,
    );
    let (running, waiting) = live_counts(&live, &summary.real_path);
    let daily = query::daily_usage_for_project(&db, summary.id, settings.chart_days as usize, now)
        .map_err(read_error)?
        .into_iter()
        .map(|d| json!({ "date": date(d.day_start_ms), "tokens": d.usage.total_tokens(), "cost_usd": usd(d.cost_usd) }))
        .collect::<Vec<_>>();
    let sessions = query::session_history(&db, summary.id)
        .map_err(read_error)?
        .into_iter()
        .map(|h| {
            json!({
                "id": h.id,
                "title": h.title,
                "live": live.iter().any(|l| l.session_id == h.id),
                "started_at": iso(h.started_at),
                "last_active_at": iso(h.last_activity_at),
                "branch": h.git_branch.filter(|b| !b.trim().is_empty()),
                "messages": h.message_count,
                "tokens": h.usage.total_tokens(),
                "cost_usd": usd(h.cost_usd),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "id": summary.id,
        "name": meta.display_name.clone().unwrap_or_else(|| dir_name(&summary.real_path)),
        "path": summary.real_path,
        "folder_exists": Path::new(&summary.real_path).is_dir(),
        "group": group_name(group),
        "pinned": meta.pinned,
        "archived": meta.archived,
        "note": meta.note.filter(|n| !n.trim().is_empty()),
        "description": readme::project_description(Path::new(&summary.real_path)),
        "sessions_count": summary.sessions,
        "tokens": summary.usage.total_tokens(),
        "cost_usd": usd(summary.cost_usd),
        "last_active_at": iso(summary.last_activity_at),
        "running": running,
        "waiting": waiting,
        "daily": daily,
        "sessions": sessions,
        "unpriced_models": unpriced_models(&db)?,
    }))
}

/// Works without an index: the live records are Claude Code's own, and only
/// the titles and project ids come from Perch.
fn live_sessions(host: &dyn Host) -> ToolResult {
    let live = host.live_sessions();
    let db = host.open_index().ok();
    let ids: Vec<&str> = live.iter().map(|l| l.session_id.as_str()).collect();
    let titles = db
        .as_ref()
        .and_then(|db| query::titles_for(db, &ids).ok())
        .unwrap_or_default();
    let projects: HashMap<String, i64> = db
        .as_ref()
        .and_then(|db| project_ids_by_path(db).ok())
        .unwrap_or_default();

    let sessions = live
        .iter()
        .map(|l| {
            let (status, since, reason) = match &l.status {
                SessionStatus::Working => ("working", l.status_updated_at, None),
                SessionStatus::Idle => ("idle", l.status_updated_at, None),
                SessionStatus::Waiting { reason, since_ms } => {
                    ("waiting", *since_ms, reason.clone())
                }
                SessionStatus::Background => ("background", l.status_updated_at, None),
                SessionStatus::Ended => ("ended", l.status_updated_at, None),
            };
            json!({
                "session_id": l.session_id,
                "title": titles.get(&l.session_id),
                "project_id": projects.get(&l.cwd),
                "cwd": l.cwd,
                "kind": l.kind,
                "status": status,
                "status_since": iso(Some(since)),
                "waiting_reason": reason,
                "started_at": iso(Some(l.started_at)),
                "claude_code_version": l.cc_version,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sessions": sessions, "index_available": db.is_some() }))
}

fn usage_summary(host: &dyn Host, arguments: &Value) -> ToolResult {
    let days = match arguments.get("days") {
        None | Some(Value::Null) => 7,
        Some(v) => v
            .as_i64()
            .filter(|d| (1..=90).contains(d))
            .ok_or("days must be an integer from 1 to 90")?,
    };
    let db = host.open_index()?;
    let now = host.now_ms();
    let since = now - days * query::DAY_MS;

    let (usage, cost) = query::usage_since(&db, since).map_err(read_error)?;
    let mut by_model = query::usage_by_model_since(&db, since).map_err(read_error)?;
    by_model.sort_by(|a, b| {
        b.1.total_tokens()
            .cmp(&a.1.total_tokens())
            .then_with(|| a.0.cmp(&b.0))
    });
    let top = query::top_projects(&db, since, 10).map_err(read_error)?;
    let daily = query::daily_usage(&db, days as usize, now).map_err(read_error)?;
    let unpriced = unpriced_models(&db)?;

    Ok(json!({
        "days": days,
        "since": iso(Some(since)),
        "totals": classes(&usage, cost),
        "by_model": by_model.iter().map(|(model, u, c)| {
            let mut v = classes(u, *c);
            v["model"] = json!(model);
            if unpriced.contains(model) {
                // No price, so no cost — not a cost of zero.
                v["cost_usd"] = Value::Null;
            }
            v
        }).collect::<Vec<_>>(),
        "unpriced_models": unpriced,
        "top_projects": top.iter().map(|(name, u, c)| json!({
            "name": name, "tokens": u.total_tokens(), "cost_usd": usd(*c)
        })).collect::<Vec<_>>(),
        "daily": daily.iter().map(|d| json!({
            "date": date(d.day_start_ms), "tokens": d.usage.total_tokens(), "cost_usd": usd(d.cost_usd)
        })).collect::<Vec<_>>(),
    }))
}

/// Token classes as the app counts them: thinking is a subset of output and is
/// not added again to the total.
fn classes(u: &TurnUsage, cost: f64) -> Value {
    json!({
        "input": u.input,
        "output": u.output,
        "cache_read": u.cache_read,
        "cache_write": u.cache_write_total(),
        "total": u.total_tokens(),
        "cost_usd": usd(cost),
    })
}

/// Models that have spent tokens but have no row in Perch's price table. Their
/// tokens are counted and their cost is not, so every `cost_usd` next to this
/// list is a lower bound — said out loud rather than left as a silent zero.
fn unpriced_models(db: &Db) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (model, usage, _) in query::usage_by_model(db).map_err(read_error)? {
        if usage.total_tokens() > 0
            && pricing::price_for(db, &model)
                .map_err(read_error)?
                .is_none()
        {
            out.push(model);
        }
    }
    out.sort();
    Ok(out)
}

fn project_ids_by_path(db: &Db) -> anyhow::Result<HashMap<String, i64>> {
    let mut stmt = db.conn().prepare("SELECT real_path, id FROM projects")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn live_counts(live: &[LiveSession], path: &str) -> (usize, usize) {
    let here = live.iter().filter(|l| l.cwd == path);
    let waiting = here
        .clone()
        .filter(|l| matches!(l.status, SessionStatus::Waiting { .. }))
        .count();
    (here.count(), waiting)
}

fn group_name(g: ProjectGroup) -> &'static str {
    match g {
        ProjectGroup::Pinned => "pinned",
        ProjectGroup::Active => "active",
        ProjectGroup::Recent => "recent",
        ProjectGroup::Archived => "archived",
    }
}

fn group_rank(g: ProjectGroup) -> u8 {
    match g {
        ProjectGroup::Pinned => 0,
        ProjectGroup::Active => 1,
        ProjectGroup::Recent => 2,
        ProjectGroup::Archived => 3,
    }
}

fn dir_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn read_error(e: anyhow::Error) -> String {
    format!("could not read Perch's index: {e}")
}

/// Estimated costs to a hundredth of a cent — more digits would claim a
/// precision the price table does not have.
fn usd(cost: f64) -> f64 {
    (cost * 10_000.0).round() / 10_000.0
}

fn iso(ms: Option<i64>) -> Value {
    ms.filter(|&t| t > 0)
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map_or(Value::Null, |t| {
            json!(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        })
}

fn date(day_start_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(day_start_ms)
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use perch_core::db;
    use perch_core::model::{SessionRecord, Turn};
    use perch_core::pricing::seed_default_prices;

    const DAY: i64 = 86_400_000;
    const NOW: i64 = 200 * DAY + 3_600_000;

    pub struct FixtureHost {
        _dir: tempfile::TempDir,
        db_path: std::path::PathBuf,
        live: Vec<LiveSession>,
        pub project_path: String,
    }

    impl FixtureHost {
        pub fn without_index() -> Self {
            let dir = tempfile::tempdir().unwrap();
            Self {
                db_path: dir.path().join("index.db"),
                _dir: dir,
                live: Vec::new(),
                project_path: String::new(),
            }
        }

        pub fn with_one_project() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let project = dir.path().join("proj");
            std::fs::create_dir(&project).unwrap();
            std::fs::write(project.join("README.md"), "# Proj\n\nA fixture project.").unwrap();
            let project_path = project.to_string_lossy().into_owned();
            let db_path = dir.path().join("index.db");
            {
                let db = db::open(&db_path).unwrap();
                seed_default_prices(&db).unwrap();
                let pid = db.upsert_project("-proj", &project_path, false).unwrap();
                for (id, last, input, branch) in [
                    ("s-old", NOW - 3 * DAY, 1_000u64, "main"),
                    ("s-new", NOW - 60_000, 4_000u64, "feature"),
                ] {
                    db.upsert_session(&SessionRecord {
                        id: id.into(),
                        project_id: pid,
                        file_path: format!("/tmp/{id}.jsonl"),
                        file_size: 0,
                        indexed_offset: 0,
                        started_at: Some(last - 1_000),
                        last_activity_at: Some(last),
                        cwd: Some(project_path.clone()),
                        git_branch: Some(branch.into()),
                        cc_version: None,
                        title: Some(format!("Title {id}")),
                        message_count: 2,
                    })
                    .unwrap();
                    db.insert_turns(
                        id,
                        &[Turn {
                            ts: last,
                            model: "claude-fable-5".into(),
                            usage: TurnUsage {
                                input,
                                output: 10,
                                ..Default::default()
                            },
                        }],
                    )
                    .unwrap();
                }
            }
            let live = vec![LiveSession {
                pid: 1,
                session_id: "s-new".into(),
                cwd: project_path.clone(),
                name: "n".into(),
                kind: "interactive".into(),
                status: SessionStatus::Waiting {
                    reason: Some("permission".into()),
                    since_ms: NOW - 5_000,
                },
                started_at: NOW - 100_000,
                status_updated_at: NOW - 5_000,
                cc_version: Some("2.1.0".into()),
                socket_path: None,
            }];
            Self {
                _dir: dir,
                db_path,
                live,
                project_path,
            }
        }
    }

    impl Host for FixtureHost {
        fn open_index(&self) -> Result<Db, String> {
            crate::open_index(&self.db_path)
        }
        fn live_sessions(&self) -> Vec<LiveSession> {
            self.live.clone()
        }
        fn settings(&self) -> Settings {
            Settings::default()
        }
        fn now_ms(&self) -> i64 {
            NOW
        }
        fn index_updated_at(&self) -> Option<i64> {
            Some(NOW - 1_000)
        }
    }

    fn run(host: &FixtureHost, name: &str, args: Value) -> Value {
        call(host, name, &args)
            .expect("known tool")
            .expect("tool succeeds")
    }

    #[test]
    fn list_projects_reports_totals_branch_description_and_live_state() {
        let host = FixtureHost::with_one_project();
        let out = run(&host, "list_projects", json!({}));
        let p = &out["projects"][0];
        assert_eq!(p["name"], "proj");
        assert_eq!(p["path"], host.project_path.as_str());
        assert_eq!(p["group"], "active");
        assert_eq!(p["description"], "A fixture project.");
        assert_eq!(p["sessions"], 2);
        assert_eq!(p["tokens"], 5_020);
        assert_eq!(p["branch"], "feature", "the newest session's branch");
        assert_eq!(p["running"], 1);
        assert_eq!(p["waiting"], 1);
        assert!(p["cost_usd"].as_f64().unwrap() > 0.0);
        assert!(p["last_active_at"].as_str().unwrap().ends_with('Z'));
        assert!(out["index_updated_at"].is_string());
    }

    #[test]
    fn get_project_by_id_and_by_path_agree_and_carry_sessions() {
        let host = FixtureHost::with_one_project();
        let id = run(&host, "list_projects", json!({}))["projects"][0]["id"].clone();
        let by_id = run(&host, "get_project", json!({ "id": id }));
        let by_path = run(
            &host,
            "get_project",
            json!({ "path": format!("{}/", host.project_path) }),
        );
        assert_eq!(by_id, by_path);
        assert_eq!(by_id["sessions"].as_array().unwrap().len(), 2);
        assert_eq!(by_id["sessions"][0]["id"], "s-new", "newest first");
        assert_eq!(by_id["sessions"][0]["title"], "Title s-new");
        assert_eq!(by_id["sessions"][0]["live"], true);
        assert_eq!(
            by_id["daily"].as_array().unwrap().len(),
            Settings::default().chart_days as usize
        );
        let daily_total: u64 = by_id["daily"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["tokens"].as_u64().unwrap())
            .sum();
        assert_eq!(daily_total, 5_020);
    }

    #[test]
    fn get_project_explains_bad_arguments() {
        let host = FixtureHost::with_one_project();
        assert!(call(&host, "get_project", &json!({})).unwrap().is_err());
        assert!(call(&host, "get_project", &json!({ "id": "x" }))
            .unwrap()
            .is_err());
        assert!(call(&host, "get_project", &json!({ "id": 999 }))
            .unwrap()
            .is_err());
    }

    #[test]
    fn live_sessions_carry_status_reason_title_and_project() {
        let host = FixtureHost::with_one_project();
        let out = run(&host, "live_sessions", json!({}));
        let s = &out["sessions"][0];
        assert_eq!(s["status"], "waiting");
        assert_eq!(s["waiting_reason"], "permission");
        assert_eq!(s["title"], "Title s-new");
        assert!(s["project_id"].is_i64());
        assert_eq!(out["index_available"], true);
    }

    #[test]
    fn live_sessions_still_answer_without_an_index() {
        let mut host = FixtureHost::without_index();
        host.live = FixtureHost::with_one_project().live;
        let out = run(&host, "live_sessions", json!({}));
        assert_eq!(out["index_available"], false);
        assert_eq!(out["sessions"][0]["status"], "waiting");
        assert!(out["sessions"][0]["title"].is_null());
    }

    #[test]
    fn usage_summary_windows_totals_and_validates_days() {
        let host = FixtureHost::with_one_project();
        let week = run(&host, "usage_summary", json!({}));
        assert_eq!(week["days"], 7);
        assert_eq!(week["totals"]["total"], 5_020);
        assert_eq!(week["by_model"][0]["model"], "claude-fable-5");
        assert_eq!(week["daily"].as_array().unwrap().len(), 7);

        let one = run(&host, "usage_summary", json!({ "days": 1 }));
        assert_eq!(
            one["totals"]["total"], 4_010,
            "only the session inside the last day"
        );

        assert_eq!(
            week["unpriced_models"],
            json!([]),
            "the fixture's model is priced"
        );
        assert!(call(&host, "usage_summary", &json!({ "days": 0 }))
            .unwrap()
            .is_err());
        assert!(call(&host, "usage_summary", &json!({ "days": 91 }))
            .unwrap()
            .is_err());
    }

    #[test]
    fn an_unpriced_model_has_no_cost_rather_than_a_zero_one() {
        let host = FixtureHost::with_one_project();
        {
            let db = perch_core::db::open(&host.db_path).unwrap();
            db.insert_turns(
                "s-new",
                &[Turn {
                    ts: NOW - 30_000,
                    model: "claude-imaginary-9".into(),
                    usage: TurnUsage {
                        input: 500,
                        ..Default::default()
                    },
                }],
            )
            .unwrap();
        }
        let out = run(&host, "usage_summary", json!({}));
        assert_eq!(out["unpriced_models"], json!(["claude-imaginary-9"]));
        let row = out["by_model"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["model"] == "claude-imaginary-9")
            .expect("the unpriced model is still listed with its tokens");
        assert_eq!(row["total"], 500);
        assert!(row["cost_usd"].is_null());
        assert_eq!(
            run(&host, "list_projects", json!({}))["unpriced_models"],
            json!(["claude-imaginary-9"])
        );
    }

    #[test]
    fn every_index_tool_says_to_open_perch_when_there_is_no_index() {
        let host = FixtureHost::without_index();
        for tool in ["list_projects", "get_project", "usage_summary"] {
            let err = call(&host, tool, &json!({ "id": 1 })).unwrap().unwrap_err();
            assert!(err.contains("open Perch once"), "{tool}: {err}");
        }
    }

    #[test]
    fn nothing_the_tools_return_is_written_into_the_index() {
        let host = FixtureHost::with_one_project();
        let before = std::fs::read(&host.db_path).unwrap();
        for tool in ["list_projects", "live_sessions", "usage_summary"] {
            run(&host, tool, json!({}));
        }
        run(&host, "get_project", json!({ "path": host.project_path }));
        assert_eq!(std::fs::read(&host.db_path).unwrap(), before);
    }
}
