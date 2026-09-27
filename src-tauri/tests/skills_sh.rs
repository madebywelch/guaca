//! skills.sh, against a scripted directory, and once against the real one.
//!
//! What is scripted is the far side: a ranking, a search, a skill's files and
//! its audits, each with the answers that have to be handled rather than
//! shown. A page in a shape this build does not know, a skill whose files
//! reach outside it, a skill that changed between being read and being added.
//! Everything on this side of the wire is the real client and the real disk.
//!
//! The `#[ignore]`d half asks the live skills.sh whether the three endpoints
//! this build reads without a token still answer in the shape it reads. They
//! are not the documented API, so this is the test that says when they moved:
//!
//! ```sh
//! cargo test --manifest-path src-tauri/Cargo.toml --test skills_sh -- --ignored
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use guac_lib::domain::skill::{Scope, SkillError};
use guac_lib::skills::{Skills, SkillsError};
use guac_lib::skills_sh::{Board, Directory, DirectoryError, Verdict};

const PDF: &str = "---\nname: pdf\ndescription: Use when a task involves a PDF.\nlicense: Proprietary\n---\n# PDF\n\nRead `reference.md` first.\n";

#[derive(Default)]
struct Far {
    /// How many times the moving skill has been downloaded.
    moves: AtomicU32,
}

async fn ranking(Path((board, page)): Path<(String, u32)>) -> Response {
    match (board.as_str(), page) {
        ("all-time", 0) => Json(json!({ "skills": [
            { "source": "anthropics/skills", "skillId": "pdf", "name": "pdf", "installs": 201834 },
            { "source": "open.feishu.cn", "skillId": "lark-doc", "name": "lark-doc", "installs": 90000 },
        ], "total": 3, "hasMore": true, "page": 0 }))
        .into_response(),
        ("all-time", 1) => Json(json!({ "skills": [
            { "source": "someone/skills", "skillId": "notes", "name": "notes", "installs": 12 },
        ], "total": 3, "hasMore": false, "page": 1 }))
        .into_response(),
        // What the documented API answers, which is not what this build reads.
        ("hot", _) => Json(json!({ "data": [], "pagination": {} })).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn search(Query(query): Query<HashMap<String, String>>) -> Response {
    match query.get("q").map(String::as_str) {
        Some("busy") => (StatusCode::TOO_MANY_REQUESTS, "slow down").into_response(),
        Some(q) => Json(json!({ "query": q, "skills": [
            { "id": "anthropics/skills/pdf", "source": "anthropics/skills", "skillId": "pdf", "name": "pdf", "installs": 201834 },
        ]}))
        .into_response(),
        None => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn download(
    State(far): State<Arc<Far>>,
    Path((owner, repo, slug)): Path<(String, String, String)>,
) -> Response {
    let files = match (owner.as_str(), repo.as_str(), slug.as_str()) {
        ("anthropics", "skills", "pdf") => json!([
            { "path": "SKILL.md", "contents": PDF },
            { "path": "reference.md", "contents": "# Reference" },
            { "path": "scripts/fill.py", "contents": "print('fill')" },
            { "path": ".github/workflows/ci.yml", "contents": "on: push" },
        ]),
        ("someone", "skills", "moving") => {
            let seen = far.moves.fetch_add(1, Ordering::SeqCst);
            json!([{ "path": "SKILL.md", "contents": format!("---\nname: moving\ndescription: d\n---\nVersion {seen}") }])
        }
        ("evil", "skills", "escape") => json!([
            { "path": "SKILL.md", "contents": PDF },
            { "path": "../../../escaped.txt", "contents": "out" },
        ]),
        ("broken", "skills", "html") => return "<!DOCTYPE html>".into_response(),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    Json(json!({ "files": files, "hash": "theirs, never trusted" })).into_response()
}

async fn audit(Path((owner, _, _)): Path<(String, String, String)>) -> Response {
    match owner.as_str() {
        "anthropics" => Json(json!({ "audits": [
            { "provider": "Gen Agent Trust Hub", "status": "pass", "summary": "No risks detected" },
            { "provider": "Snyk", "status": "warn", "summary": "Runs local scripts" },
            { "provider": "Someone New", "status": "probably-fine", "summary": "?" },
        ]}))
        .into_response(),
        "broken" => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn scripted() -> Directory {
    let app = Router::new()
        .route("/api/skills/:board/:page", get(ranking))
        .route("/api/search", get(search))
        .route("/api/download/:owner/:repo/:slug", get(download))
        .route("/api/v1/skills/audit/:owner/:repo/:slug", get(audit))
        .with_state(Arc::new(Far::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Directory::at(&format!("http://{addr}"))
}

fn disk() -> (Skills, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Skills::new(dir.path().join("skills")), dir)
}

#[tokio::test]
async fn a_ranking_in_a_shape_this_build_does_not_know_is_an_error_and_not_an_empty_list() {
    let directory = scripted().await;
    let refused = directory.board(Board::Hot, 0).await.unwrap_err();
    assert!(matches!(refused, DirectoryError::Unexpected(_)), "{refused}");
    assert!(refused.to_string().contains("skills.sh"), "{refused}");
}

#[tokio::test]
async fn a_busy_directory_a_short_search_and_a_non_json_answer_each_say_which_they_are() {
    let directory = scripted().await;
    assert!(matches!(directory.search("busy").await, Err(DirectoryError::Busy)));
    assert!(matches!(directory.search(" p ").await, Err(DirectoryError::ShortQuery)));
    let html = directory.preview("broken/skills/html").await.unwrap_err();
    assert!(matches!(html, DirectoryError::Unexpected(_)), "{html}");
    let gone = directory.preview("anthropics/skills/nothing").await.unwrap_err();
    assert!(matches!(gone, DirectoryError::Missing(_)), "{gone}");
    let site = directory.preview("open.feishu.cn/lark-doc").await.unwrap_err();
    assert!(matches!(site, DirectoryError::NotAddable(_)), "{site}");
}

#[tokio::test]
async fn a_skill_whose_files_reach_outside_it_is_refused_and_nothing_is_written() {
    let directory = scripted().await;
    let (skills, dir) = disk();
    let refused = directory.preview("evil/skills/escape").await.unwrap_err();
    assert!(
        matches!(refused, DirectoryError::Invalid(SkillError::BadFile(ref path)) if path.contains("..")),
        "{refused}"
    );
    let added = directory.add(&skills, Scope::Workspace, "evil/skills/escape", "any").await;
    assert!(matches!(added, Err(DirectoryError::Invalid(SkillError::BadFile(_)))));
    assert!(!dir.path().join("escaped.txt").exists());
    assert!(skills.in_scope(Scope::Workspace).is_empty());
}

#[tokio::test]
async fn a_skill_that_changed_after_it_was_read_is_not_added() {
    let directory = scripted().await;
    let (skills, _dir) = disk();
    let read = directory.preview("someone/skills/moving").await.unwrap();
    assert!(read.body.contains("Version 0"));
    let refused = directory.add(&skills, Scope::Workspace, &read.id, &read.hash).await.unwrap_err();
    assert!(matches!(refused, DirectoryError::Changed), "{refused}");
    assert!(skills.in_scope(Scope::Workspace).is_empty());
}

#[tokio::test]
async fn the_ranking_is_read_a_page_at_a_time_and_a_site_published_skill_is_listed_but_not_addable()
{
    let directory = scripted().await;
    let first = directory.board(Board::AllTime, 0).await.unwrap();
    assert!(first.has_more);
    let ids: Vec<_> = first.skills.iter().map(|listing| listing.id.as_str()).collect();
    assert_eq!(ids, ["anthropics/skills/pdf", "open.feishu.cn/lark-doc"]);
    assert_eq!(first.skills[0].installs, 201_834);
    assert!(first.skills[0].addable && !first.skills[1].addable);
    let second = directory.board(Board::AllTime, 1).await.unwrap();
    assert!(!second.has_more);
    assert_eq!(second.skills[0].id, "someone/skills/notes");
    let found = directory.search("pdf").await.unwrap();
    assert_eq!(found[0].id, "anthropics/skills/pdf");
}

#[tokio::test]
async fn a_preview_shows_the_body_the_files_and_every_verdict_including_one_it_does_not_know() {
    let directory = scripted().await;
    let read = directory.preview("anthropics/skills/pdf").await.unwrap();
    assert_eq!(read.name, "pdf");
    assert_eq!(read.description, "Use when a task involves a PDF.");
    assert!(read.body.contains("reference.md"));
    assert_eq!(
        read.files,
        ["reference.md", "scripts/fill.py"],
        "the repository's own files stay behind"
    );
    assert_eq!(read.url, "https://skills.sh/anthropics/skills/pdf");
    assert_ne!(read.hash, "theirs, never trusted");
    let verdicts: Vec<_> = read.audits.unwrap().iter().map(|audit| audit.verdict).collect();
    assert_eq!(verdicts, [Verdict::Pass, Verdict::Warn, Verdict::Unknown]);

    // None run yet, and none readable, are drawn differently.
    let quiet = directory.preview("someone/skills/moving").await.unwrap();
    assert_eq!(quiet.audits, Some(Vec::new()));
}

#[tokio::test]
async fn an_added_skill_lands_whole_with_where_it_came_from_and_is_not_added_twice() {
    let directory = scripted().await;
    let (skills, _dir) = disk();
    let read = directory.preview("anthropics/skills/pdf").await.unwrap();
    let added = directory.add(&skills, Scope::Workspace, &read.id, &read.hash).await.unwrap();
    assert_eq!(added.name, "pdf");
    assert_eq!(added.files, read.files);
    assert_eq!(added.origin.as_deref(), Some("https://skills.sh/anthropics/skills/pdf"));
    assert_eq!(skills.read_file(None, "pdf", "reference.md").unwrap(), "# Reference");

    let again = directory.add(&skills, Scope::Workspace, &read.id, &read.hash).await.unwrap_err();
    assert!(matches!(again, DirectoryError::Store(SkillsError::Taken(_))), "{again}");
}

// ---- live ------------------------------------------------------------------

#[tokio::test]
#[ignore = "asks the live skills.sh"]
async fn skills_sh_still_answers_the_way_this_build_reads_it() {
    let directory = Directory::new();
    let top = directory.board(Board::AllTime, 0).await.expect("the ranking");
    assert!(top.skills.len() >= 50, "a page of the ranking: {}", top.skills.len());
    assert!(top.has_more);
    let installs: Vec<u64> = top.skills.iter().map(|listing| listing.installs).collect();
    assert!(
        installs.windows(2).all(|pair| pair[0] >= pair[1]),
        "most installed first: {installs:?}"
    );
    for board in [Board::Trending, Board::Hot] {
        assert!(!directory.board(board, 0).await.expect("a board").skills.is_empty());
    }
    let found = directory.search("pdf").await.expect("a search");
    assert!(found.iter().any(|listing| listing.addable), "{found:?}");

    let first = top.skills.iter().find(|listing| listing.addable).expect("one addable");
    let read = directory.preview(&first.id).await.expect("a preview");
    assert!(!read.body.is_empty() && !read.description.is_empty(), "{read:?}");
    assert!(read.audits.is_some(), "the audit endpoint still answers without a token");
    let (skills, _dir) = disk();
    let added =
        directory.add(&skills, Scope::Workspace, &read.id, &read.hash).await.expect("added");
    assert_eq!(added.files, read.files);
}
