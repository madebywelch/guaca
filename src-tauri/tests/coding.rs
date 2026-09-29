//! Which program writes the code, end to end.
//!
//! Everything else about a coding job is covered where it lives: the two
//! parsers have unit tests beside them, and the store has one for the column.
//! What none of those can see is the seam this suite exists for, which is that
//! the harness named on a *repository* is the program that actually gets
//! started in it. Read the column, drop the value, and every suite in this repo
//! still passes while every job in every workspace runs the harness the
//! operator moved away from.
//!
//! ## Why there is a program on `PATH` here and not a mock
//!
//! Because the thing being tested is a process. `coding::run` spawns a binary
//! by name, reads its stdout as one JSON object per line, and folds those into
//! an outcome; a fake in front of that would be a test of the fold, which
//! already has one. So the stand-ins below are real executables, found on
//! `PATH` the way the real ones are, and each records the argument vector it
//! was handed. The one thing that cannot be checked this way is whether the
//! real CLI still accepts that vector, and that is what the `#[ignore]`d tests
//! at the bottom are for: the same failure shape `subscription.rs` and
//! `plugins.rs` keep a live half for.

mod harness;

use std::path::{Path, PathBuf};

use guac_lib::coding::{self, Progress};
use guac_lib::domain::approval::Decision;
use guac_lib::domain::terminal::{Gate, Harness as Which, Payer, Tuning};
use guac_lib::runtime::events::UiEvent;
use guac_lib::runtime::guard::GuardLimits;
use guac_lib::runtime::{Continued, Origin};

use harness::*;

/// Where a stand-in records what it was called with. Inside the repository it
/// was run in, which is what makes it per-test: two tests run concurrently in
/// one binary and share one `PATH`.
const ARGV: &str = ".argv";

/// What a stand-in prints, if the test wrote one. Otherwise it answers with a
/// canned success.
const SAY: &str = ".say";

/// What it exits with. A file rather than an environment variable, because the
/// environment is process-wide and these tests run concurrently: a test asking
/// for a non-zero exit would be asking it of whatever else was running.
const EXIT: &str = ".exit";

/// How long the stand-in waits before it answers.
///
/// Everything else here is about a job that has finished. A job that can be
/// *reached* has to still be running when the test reaches it, and the only
/// honest way to arrange that against a real process is to make it slow.
const LINGER: &str = ".linger";

/// A directory holding the three stand-ins, put on `PATH` exactly once.
///
/// Once, because `PATH` is process-wide and these tests run concurrently:
/// writing it per test is a read racing a write in another thread. Written
/// before any test body runs anything that looks it up, and never again.
fn stand_ins() -> &'static Path {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        for (name, script) in [
            ("claude", include_str!("fixtures/claude.py")),
            ("codex", include_str!("fixtures/codex.py")),
            ("pi", include_str!("fixtures/pi.py")),
        ] {
            let at = dir.path().join(name);
            std::fs::write(&at, script).unwrap();
            std::fs::set_permissions(&at, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
        }
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{path}", dir.path().display()));
        dir
    });
    dir.path()
}

#[tokio::test]
async fn a_noisy_harness_cannot_fill_stderr_and_deadlock() {
    stand_ins();
    let repo = a_repository("noisy");
    std::fs::write(repo.join(".noisy"), "").unwrap();
    let done = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run_job(Which::Codex, &repo, "work", |_| {}),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(done.failed.is_none());
    let _ = std::fs::remove_dir_all(repo);
}

#[tokio::test]
async fn codex_runs_in_the_repository_and_retains_its_own_session() {
    stand_ins();
    let repo = a_repository("codex");
    let mut progress = Vec::new();
    let outcome = run_job(Which::Codex, &repo, "fix it", |p| progress.push(p)).await.unwrap();
    assert_eq!(outcome.said, "Fixed the flaky test and pushed.");
    assert_eq!(outcome.tool_calls, 1);
    assert_eq!(outcome.session_id, "codex-session");
    assert!(outcome.failed.is_none());
    assert!(outcome.cost.is_none());
    let argv = argv_at(&repo);
    assert_eq!(argv[0], "app-server");
    let requests = std::fs::read_to_string(repo.join(".rpc.jsonl")).unwrap();
    assert!(requests.contains("Commit early and often"));
    assert!(requests.contains("fix it"));
    assert!(!argv.contains(&"--model".into()));
    assert_eq!(progress.len(), 2);
    let _ = std::fs::remove_dir_all(repo);
}

#[tokio::test]
async fn codex_without_auth_refuses_before_starting_a_thread_or_spending_a_model_call() {
    stand_ins();
    let repo = a_repository("codex-signed-out");
    std::fs::write(repo.join(".codex_signed_out"), "").unwrap();
    let error = run_job(Which::Codex, &repo, "work", |_| {}).await.unwrap_err().to_string();
    assert!(error.contains("Codex is not signed in on this backend"), "{error}");
    assert!(error.contains("codex login --device-auth"), "{error}");
    let requests = std::fs::read_to_string(repo.join(".rpc.jsonl")).unwrap();
    assert!(requests.contains("account/read"));
    assert!(!requests.contains("thread/start"));
    assert!(!requests.contains("turn/start"));
    let _ = std::fs::remove_dir_all(repo);
}

#[tokio::test]
async fn codex_custom_provider_does_not_require_an_openai_account() {
    stand_ins();
    let repo = a_repository("codex-custom-provider");
    std::fs::write(repo.join(".codex_custom_provider"), "").unwrap();
    let outcome = run_job(Which::Codex, &repo, "work", |_| {}).await.unwrap();
    assert!(outcome.failed.is_none());
    assert_eq!(outcome.tool_calls, 1);
    let _ = std::fs::remove_dir_all(repo);
}

/// A real git repository, because that is what a linked one has to be, and
/// because the stand-in records into it.
fn a_repository(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("guac-coding-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let done = std::process::Command::new("git").arg("-C").arg(&root).arg("init").output().unwrap();
    assert!(done.status.success(), "git has to be installed to run this suite");
    std::fs::canonicalize(&root).unwrap()
}

/// Git with an identity of its own, so the suite does not depend on what the
/// machine running it has configured and does not try to sign.
fn git(root: &Path, args: &[&str]) {
    let done = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=guac",
            "-c",
            "user.email=guac@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(done.status.success(), "git {args:?} failed: {done:?}");
}

/// A repository sitting where the last job left it: on a branch whose work is
/// already in `main`. The state an operator finds weeks later and the reason
/// a job is told its footing at all.
fn a_repository_on_a_landed_branch(name: &str) -> PathBuf {
    let root = a_repository(name);
    git(&root, &["checkout", "-b", "main"]);
    std::fs::write(root.join("a.txt"), "one").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "one"]);
    git(&root, &["checkout", "-b", "landed"]);
    root
}

/// Gives an agent a terminal whose whole directory is this repository, and
/// answers with where it now is.
///
/// Moved into place rather than cloned: the stand-ins read their instructions
/// out of files the test wrote into the repository, and record their argument
/// vector into whatever directory they were started in. A job started with no
/// `directory` runs at the top of the terminal, so this is the arrangement in
/// which every test above reads what it wrote. The ordinary one, a repository
/// in a directory of its own inside the terminal, is
/// `a_job_runs_in_the_directory_it_names_inside_the_agents_terminal`.
fn give_terminal(h: &Harness, agent: &str, repo: PathBuf, which: Which, gate: Gate) -> PathBuf {
    let card = h.agent_named(agent).unwrap();
    h.runtime.store().set_has_terminal(card.id, true).unwrap();
    h.runtime.store().set_agent_coding(card.id, which, gate).unwrap();
    let home = h.runtime.terminals().dir(card.id);
    std::fs::create_dir_all(home.parent().unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&home);
    if std::fs::rename(&repo, &home).is_err() {
        // Across filesystems a rename fails, and a copy is the same tree.
        let copied =
            std::process::Command::new("cp").arg("-R").arg(&repo).arg(&home).status().unwrap();
        assert!(copied.success(), "could not move {repo:?} into the terminal");
        let _ = std::fs::remove_dir_all(&repo);
    }
    std::fs::canonicalize(&home).unwrap()
}

/// Runs a job the way the runtime does, with nothing steering it and nothing
/// granted.
async fn run_job(
    which: Which,
    dir: &Path,
    brief: &str,
    watching: impl FnMut(Progress) + Send,
) -> Result<coding::Outcome, coding::CodingError> {
    run_job_env(which, dir, brief, &guac_lib::secrets::Environment::default(), watching).await
}

/// The same, with secrets granted.
async fn run_job_env(
    which: Which,
    dir: &Path,
    brief: &str,
    env: &guac_lib::secrets::Environment,
    watching: impl FnMut(Progress) + Send,
) -> Result<coding::Outcome, coding::CodingError> {
    let (_controls, controlled) = tokio::sync::mpsc::channel(8);
    let (signals, _heard) = tokio::sync::mpsc::channel(8);
    // A fresh UUID, as the runtime mints one: the real Claude Code refuses a
    // session id that is not a UUID, and one that is already in use.
    let session = match which {
        Which::Codex => String::new(),
        _ => uuid::Uuid::new_v4().to_string(),
    };
    coding::run(
        coding::Job {
            session: &session,
            ..job(which, dir.to_str().unwrap(), brief, Gate::Open, env)
        },
        controlled,
        signals,
        watching,
    )
    .await
}

/// The program's own model and effort, which is what a job runs on when the
/// operator chose nothing.
static NO_TUNING: Tuning = Tuning { model: None, effort: None, pays: Payer::Own };

/// One job, as the runtime would describe it. Codex names its own thread, so
/// it gets no session; the other two are started with one.
fn job<'a>(
    which: Which,
    dir: &'a str,
    brief: &'a str,
    gate: Gate,
    env: &'a guac_lib::secrets::Environment,
) -> coding::Job<'a> {
    coding::Job {
        harness: which,
        directory: dir,
        brief,
        session: if which == Which::Codex { "" } else { "test-session" },
        resume: false,
        gate,
        env,
        bridge: None,
        tuning: &NO_TUNING,
        lent: None,
    }
}

/// The brief a stand-in in this directory was handed, however its program
/// takes one: Claude Code as its first stdin message, Codex as a `turn/start`
/// input, pi as an RPC `prompt`. Empty until the program has been handed one, so a
/// test can wait on it.
fn brief_seen(which: Which, dir: &Path) -> String {
    let recorded = match which {
        Which::Claude => ".claude_prompt",
        Which::Codex => ".rpc.jsonl",
        Which::Pi => ".pi_prompt",
    };
    std::fs::read_to_string(dir.join(recorded)).unwrap_or_default()
}

/// Every argument the stand-in in this repository was handed.
fn argv_at(repository: &Path) -> Vec<String> {
    let raw = std::fs::read_to_string(repository.join(ARGV)).expect("the stand-in never ran");
    raw.split("\n<<>>\n").map(|arg| arg.to_string()).filter(|arg| !arg.is_empty()).collect()
}

// ---- the seam ------------------------------------------------------------

#[tokio::test]
async fn a_repository_set_to_claude_starts_claude_and_not_the_other_one() {
    stand_ins();
    let repo = a_repository("claude");

    let outcome = run_job(Which::Claude, &repo, "fix the flaky test", |_| {}).await.unwrap();

    let argv = argv_at(&repo);
    // The brief reaches the program whole, as its first message: the harness
    // cannot see the conversation it came from. On stdin rather than the
    // command line, because stdin is where a stop is sent.
    assert!(brief_seen(Which::Claude, &repo).contains("fix the flaky test"));
    assert!(!argv.iter().any(|arg| arg.contains("fix the flaky test")), "{argv:?}");
    // Claude Code's own vector, and this is the half no unit test can check:
    // the CLI refuses `stream-json` without `--verbose`, which is a job that
    // never starts rather than a job that fails.
    let after = |flag: &str| argv.iter().position(|arg| arg == flag).map(|at| argv[at + 1].clone());
    assert_eq!(after("--input-format").as_deref(), Some("stream-json"), "{argv:?}");
    assert_eq!(after("--output-format").as_deref(), Some("stream-json"), "{argv:?}");
    assert!(argv.contains(&"--verbose".to_string()), "{argv:?}");
    assert!(argv.contains(&"bypassPermissions".to_string()), "{argv:?}");
    // And pi's, which would mean nothing to it.
    assert!(!argv.contains(&"--mode".to_string()), "{argv:?}");

    assert_eq!(outcome.said, "Fixed the flaky test and pushed.");
    assert_eq!(outcome.tool_calls, 1);
    assert_eq!(outcome.model, "claude-opus-5");
    assert_eq!(outcome.cost, Some(0.12));
    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn a_repository_set_to_pi_starts_pi() {
    stand_ins();
    let repo = a_repository("pi");

    let mut seen = Vec::new();
    let outcome = run_job(Which::Pi, &repo, "fix the flaky test", |p| seen.push(p)).await.unwrap();

    let argv = argv_at(&repo);
    assert!(argv.contains(&"--mode".to_string()), "{argv:?}");
    assert!(argv.contains(&"rpc".to_string()), "{argv:?}");
    assert!(!argv.contains(&"stream-json".to_string()), "{argv:?}");
    // Over the protocol rather than the command line, which is what lets the
    // same process take a correction while it works.
    assert!(brief_seen(Which::Pi, &repo).contains("fix the flaky test"));
    assert_eq!(outcome.said, "Fixed the flaky test and pushed.");
    let named = argv.iter().position(|arg| arg == "--session-id").map(|at| argv[at + 1].clone());
    assert_eq!(Some(outcome.session_id.clone()), named, "the session it was started as");
    assert_eq!(outcome.cost, Some(0.12));
    assert_eq!(outcome.model, "gpt-5.6");
    // The watcher is the panel in the channel, and it is fed from the stream
    // rather than from the outcome: a job that says nothing for twenty minutes
    // is what this exists to prevent.
    assert_eq!(
        seen.first(),
        Some(&Progress::Using { tool: "bash".into(), detail: "npm test".into() })
    );
    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn both_harnesses_are_given_the_same_standing_instruction() {
    // The prompt that says nobody will answer and commits are the only undo is
    // not one harness's. Appended to whichever runs, or a job started on the
    // other one silently loses every checkpoint the operator has.
    stand_ins();
    for (which, name) in [(Which::Pi, "prompt-pi"), (Which::Claude, "prompt-claude")] {
        let repo = a_repository(name);
        run_job(which, &repo, "do the thing", |_| {}).await.unwrap();
        let argv = argv_at(&repo);
        assert!(argv.contains(&"--append-system-prompt".to_string()), "{which:?}: {argv:?}");
        assert!(
            argv.iter().any(|arg| arg.contains("Commit early and often")),
            "{which:?}: {argv:?}"
        );
        let _ = std::fs::remove_dir_all(&repo);
    }
}

#[tokio::test]
async fn a_harness_that_reports_a_failed_turn_is_not_a_job_with_nothing_to_do() {
    // The afternoon this cost, from both ends. Each program reports a spent
    // credential inside its own stream and exits zero about it, so a job that
    // never ran arrives looking exactly like a job that found nothing to change.
    stand_ins();
    for (which, name, stream, exit) in [
        // `pi` reports it and exits zero, which is the shape that cost the
        // afternoon.
        (
            Which::Pi,
            "spent-pi",
            r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"You're out of extra usage."}}"#,
            "0",
        ),
        // Claude Code reports it and exits non-zero, which is the shape that
        // would otherwise be reported as `exit 1` with the reason thrown away:
        // a stream that said why beats an exit code that did not.
        (
            Which::Claude,
            "spent-claude",
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"You're out of extra usage."}"#,
            "1",
        ),
    ] {
        let repo = a_repository(name);
        std::fs::write(repo.join(SAY), format!("{stream}\n")).unwrap();
        std::fs::write(repo.join(EXIT), exit).unwrap();

        let outcome = run_job(which, &repo, "do the thing", |_| {})
            .await
            .expect("a stream that reports its own failure is not a dead process");
        let why = outcome.failed.unwrap_or_else(|| panic!("{which:?} reported a silent no-op"));
        assert!(why.contains("out of extra usage"), "{which:?}: {why}");
        assert!(outcome.said.is_empty(), "{which:?}: an errored run carries no answer");
        let _ = std::fs::remove_dir_all(&repo);
    }
}

#[tokio::test]
async fn a_harness_that_dies_without_answering_says_so_rather_than_reporting_success() {
    stand_ins();
    let repo = a_repository("dead");
    std::fs::write(repo.join(SAY), "not json at all\n").unwrap();
    std::fs::write(repo.join(EXIT), "3").unwrap();

    let err = run_job(Which::Pi, &repo, "do the thing", |_| {})
        .await
        .expect_err("nothing was said and the process failed");
    assert!(err.to_string().contains("exit 3"), "{err}");
    let _ = std::fs::remove_dir_all(&repo);
}

/// Both are found by name on `PATH`, which is what the panel offering the
/// choice asks before it draws it.
///
/// Only the positive half. The negative is a missing binary, which means a
/// different `PATH`, and `PATH` is process-wide while these run concurrently.
/// It is covered where it is cheap and where it matters: `TerminalPanel`'s
/// suite draws the choice disabled with the install command under it.
#[tokio::test]
async fn a_harness_on_this_machine_is_found_by_name() {
    stand_ins();
    assert!(coding::presence(Which::Pi).await.installed());
    assert!(coding::presence(Which::Claude).await.installed());
}

/// A harness too old for the bridge is still a harness.
///
/// The stand-ins print `stand-in` for `--version`, which carries no number at
/// all, so this is also the unreadable case: both have to leave the program
/// usable and turn only the bridge off. The other direction would wire a job to
/// a contract nothing has ever checked, on the strength of a version string
/// nobody could parse.
#[tokio::test]
async fn a_version_nothing_can_read_runs_the_job_without_a_bridge() {
    stand_ins();
    match coding::presence(Which::Claude).await {
        coding::Presence::Installed { bridged, version } => {
            assert!(!bridged, "an unreadable version cannot claim the contract holds");
            assert_eq!(version, "stand-in", "the program's own answer, not a parse of it");
        }
        other => panic!("{other:?}"),
    }
    // `pi` at the version its RPC was measured against is reachable, which is
    // what the panel reads to offer a correction box at all.
    assert!(matches!(
        coding::presence(Which::Pi).await,
        coding::Presence::Installed { bridged: true, .. }
    ));
}

#[tokio::test]
async fn codex_acknowledges_steering_and_rejects_completion_races() {
    stand_ins();
    for (mode, accepted) in
        [("", true), (".codex_reject_steer", false), (".codex_finish_before_ack", false)]
    {
        let repo = a_repository(&format!("steer{mode}"));
        std::fs::write(repo.join(".codex_hold"), "").unwrap();
        if !mode.is_empty() {
            std::fs::write(repo.join(mode), "").unwrap();
        }
        let path = repo.to_string_lossy().to_string();
        let (sender, controlled) = tokio::sync::mpsc::channel(8);
        let (signals, _) = tokio::sync::mpsc::channel(8);
        let job = tokio::spawn(async move {
            let env = guac_lib::secrets::Environment::default();
            coding::run(
                job(Which::Codex, &path, "work", Gate::Open, &env),
                controlled,
                signals,
                |_| {},
            )
            .await
        });
        let (reply, answer) = tokio::sync::oneshot::channel();
        sender
            .send(coding::Control::Steer { message: "Fix the tests first".into(), reply })
            .await
            .unwrap();
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(5), answer).await.unwrap().unwrap();
        assert_eq!(result.is_ok(), accepted, "{result:?}");
        if mode == ".codex_reject_steer" {
            assert!(!job.is_finished(), "a rejected correction must not stop an active job");
            job.abort();
            let _ = job.await;
        } else {
            let out = job.await.unwrap().unwrap();
            assert!(out.failed.is_none());
        }
        let log = std::fs::read_to_string(repo.join(".rpc.jsonl")).unwrap();
        let messages: Vec<serde_json::Value> =
            log.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(
            messages.iter().filter(|m| m["method"] == "turn/start").count(),
            1,
            "steering never starts a replacement turn"
        );
        let request = messages.iter().find(|m| m["method"] == "turn/steer").unwrap();
        assert_eq!(request["params"]["expectedTurnId"], "codex-turn");
        assert_eq!(request["params"]["threadId"], "codex-session");
        let _ = std::fs::remove_dir_all(repo);
    }
}

#[tokio::test]
async fn codex_keeps_steering_while_the_gate_is_waiting() {
    stand_ins();
    for allow in [true, false] {
        let repo = a_repository(&format!("codex-gate-{allow}"));
        std::fs::write(repo.join(".codex_gate"), "").unwrap();
        std::fs::write(repo.join(".codex_command"), "./ship.sh").unwrap();
        std::fs::write(repo.join("ship.sh"), "#!/bin/sh\ngit push origin HEAD\n").unwrap();
        let path = repo.to_string_lossy().to_string();
        let (sender, controlled) = tokio::sync::mpsc::channel(8);
        let (signals, mut heard) = tokio::sync::mpsc::channel(8);
        let job = tokio::spawn(async move {
            let env = guac_lib::secrets::Environment::default();
            coding::run(
                job(Which::Codex, &path, "work", Gate::AskBeforePushing, &env),
                controlled,
                signals,
                |_| {},
            )
            .await
        });
        // The thread comes first, and it is what a follow-up resumes.
        let signal = loop {
            let signal = tokio::time::timeout(std::time::Duration::from_secs(5), heard.recv())
                .await
                .unwrap()
                .unwrap();
            if !matches!(signal, coding::Signal::Session(_)) {
                break signal;
            }
        };
        let coding::Signal::Permission { line, reply: decision, .. } = signal else {
            panic!("wrong signal")
        };
        assert_eq!(line, "./ship.sh");
        assert!(!job.is_finished());
        assert!(!repo.join(".pushed").exists());
        let (reply, answer) = tokio::sync::oneshot::channel();
        sender
            .send(coding::Control::Steer {
                message: "Check the tests before pushing".into(),
                reply,
            })
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), answer)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!repo.join(".pushed").exists(), "steering must not answer a pending approval");
        decision.send(allow).unwrap();
        job.await.unwrap().unwrap();
        assert_eq!(repo.join(".pushed").exists(), allow);
        assert_eq!(
            std::fs::read_to_string(repo.join(".verdict")).unwrap(),
            if allow { "accept" } else { "decline" }
        );
        let _ = std::fs::remove_dir_all(repo);
    }
}

#[tokio::test]
async fn codex_requires_its_selected_policy_and_reports_truncated_or_failed_turns() {
    stand_ins();
    for mode in [".codex_bad_policy", ".codex_early", ".codex_failure"] {
        let repo = a_repository(mode);
        std::fs::write(repo.join(mode), "").unwrap();
        let (_, controlled) = tokio::sync::mpsc::channel(8);
        let (signals, _) = tokio::sync::mpsc::channel(8);
        let env = guac_lib::secrets::Environment::default();
        let path = repo.to_string_lossy().to_string();
        let result = coding::run(
            job(Which::Codex, &path, "work", Gate::AskBeforePushing, &env),
            controlled,
            signals,
            |_| {},
        )
        .await;
        if mode == ".codex_failure" {
            assert_eq!(result.unwrap().failed.as_deref(), Some("fixture failed after editing"));
        } else {
            assert!(result.is_err(), "{result:?}");
        }
        if mode == ".codex_bad_policy" {
            assert!(!std::fs::read_to_string(repo.join(".rpc.jsonl"))
                .unwrap()
                .contains("turn/start"));
        }
        let _ = std::fs::remove_dir_all(repo);
    }
}

// ---- the whole path ------------------------------------------------------

/// The public correction call reaches a Codex job while its push is parked
/// on the operator's desk. A denied card settles the CLI request and job.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_operator_can_steer_codex_while_its_push_waits_for_a_decision() {
    stand_ins();
    let repo = a_repository("codex-runtime-steering");
    std::fs::write(repo.join(".codex_gate"), "").unwrap();
    let stub = serve(|body| {
        if anyone_said(body, "has finished") {
            Script::Say("The coding job returned.".into())
        } else {
            Script::Code("fix the flaky test".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let engineer = h.agent_named("Engineer").unwrap();
    let repo = give_terminal(&h, "Engineer", repo, Which::Codex, Gate::AskBeforePushing);
    let run = h.runtime.send_from_human(engineer.id, "fix the flaky test").unwrap();
    h.settle(run).await;
    let request = h.awaited_request().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        h.runtime.message_job(engineer.id, "use staging"),
    )
    .await
    .expect("steering must not wait for the approval")
    .unwrap();
    h.wait_until("the correction is read", |_| repo.join(".steered").exists()).await;
    assert_eq!(std::fs::read_to_string(repo.join(".steered")).unwrap(), "use staging");
    assert!(!repo.join(".verdict").exists());
    h.runtime.decide_approval(request, Decision::Deny).unwrap();
    h.wait_until("the coding job returns", |h| {
        h.channel_texts("Engineer").iter().any(|line| line.contains("coding job returned"))
    })
    .await;
    assert_eq!(std::fs::read_to_string(repo.join(".verdict")).unwrap(), "decline");
    assert!(!repo.join(".pushed").exists());
    assert!(h.runtime.store().pending_approvals(10).unwrap().is_empty());
    assert!(h.runtime.stop_job(engineer.id, Origin::Operator).is_err(), "it is over");
    let _ = std::fs::remove_dir_all(repo);
}

/// The agent that asked is told what the harness said, in its own channel.
///
/// This is the test that reads the column. A `code` call in a repository set to
/// Claude Code has to start `claude`, and the answer has to come back to the
/// agent as a message on a fresh run, minutes after the turn that asked for it
/// ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_is_told_what_the_harness_it_was_given_said() {
    stand_ins();
    for which in Which::ALL {
        let repo = a_repository(&format!("end-to-end-{}", which.as_str()));

        // The second call is the agent reading the finished job back. Branching on
        // what it was sent rather than on a counter: a turn can take more than one
        // call, and a counter would make this depend on how many.
        let stub = serve(|body| {
            if anyone_said(body, "has finished") {
                Script::Say("The coding agent fixed the flaky test and pushed.".into())
            } else {
                Script::Code("fix the flaky test".into())
            }
        })
        .await;
        let h = harness(&stub, &["Engineer"], GuardLimits::default());

        let repo = give_terminal(&h, "Engineer", repo, which, Gate::Open);

        let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
        h.settle(run).await;

        // The job outlives the turn that started it, which is the whole shape of
        // this feature: the tool returns as soon as the process is up.
        h.wait_until("the coding job is reported back", |h| {
            h.channel_texts("Engineer").iter().any(|line| line.contains("fixed the flaky test"))
        })
        .await;

        let argv = argv_at(&repo);
        match which {
            Which::Claude => assert!(argv.contains(&"stream-json".to_string())),
            Which::Codex => assert_eq!(argv[0], "app-server"),
            Which::Pi => assert!(argv.contains(&"--mode".to_string())),
        }
        // Contained rather than equal: the brief a job is started with carries the
        // footing in front of it, which the test below is the test of.
        assert!(brief_seen(which, &repo).contains("fix the flaky test"), "{which:?}");
        let _ = std::fs::remove_dir_all(&repo);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_claude_login_failure_names_claude_and_the_next_job_uses_the_saved_switch() {
    stand_ins();
    let repo = a_repository("switch-after-login-failure");
    std::fs::write(repo.join(SAY),
        r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in. Please run /login"}"#,
    ).unwrap();
    let start = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let kickoff = start.clone();
    let stub = serve(move |_| {
        if kickoff.swap(false, std::sync::atomic::Ordering::SeqCst) {
            Script::Code("inspect the repository".into())
        } else {
            Script::Say("Received the result.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Content Marketer"], GuardLimits::default());
    let agent = h.agent_named("Content Marketer").unwrap();
    let repo = give_terminal(&h, "Content Marketer", repo, Which::Claude, Gate::Open);

    let run = h.runtime.send_from_human(agent.id, "inspect the repository").unwrap();
    h.settle(run).await;
    h.wait_until("the failed job's reply settles", |h| {
        h.channel_texts("Content Marketer").iter().any(|text| text.contains("could not finish"))
    })
    .await;
    let reported = h
        .runtime
        .store()
        .channel_messages(agent.id, 200)
        .unwrap()
        .into_iter()
        .find(|e| e.plain_text().contains("could not finish"))
        .unwrap();
    h.settle(reported.run_id).await;
    let failure = h
        .channel_texts("Content Marketer")
        .into_iter()
        .find(|text| text.contains("could not finish"))
        .unwrap();
    assert!(failure.contains("Claude Code"), "the agent must know which sign-in failed: {failure}");
    assert!(failure.contains("/login"));

    h.runtime.store().set_agent_coding(agent.id, Which::Codex, Gate::Open).unwrap();
    start.store(true, std::sync::atomic::Ordering::SeqCst);
    let run = h.runtime.send_from_human(agent.id, "retry with the saved harness").unwrap();
    h.settle(run).await;
    h.wait_until("Codex finishes in the same runtime", |h| {
        h.channel_texts("Content Marketer").iter().any(|text| text.contains("has finished"))
    })
    .await;
    assert_eq!(argv_at(&repo)[0], "app-server");
    let completion = h
        .channel_texts("Content Marketer")
        .into_iter()
        .find(|text| text.contains("has finished"))
        .unwrap();
    assert!(completion.contains("Codex"), "{completion}");
    let _ = std::fs::remove_dir_all(&repo);
}

/// The one announcement this app asks for, and the one thing that outlives the
/// message announcing it.
///
/// A turn that closes on work it has not done is given its round back, because
/// nothing of an agent's runs after its message. A started `code` job is the
/// exception and has to be: the job is already running on a process of its own
/// and comes back as its own envelope, so "started it, checking back" is a
/// report of a call that has already been made. Without the exemption the
/// nudge fires on exactly the shape `## Your repository` tells an agent to
/// write, and every coding job in the app buys a wasted model call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_started_job_is_not_an_unbacked_promise() {
    stand_ins();
    let repo = a_repository("announced");

    let stub = serve(|body| {
        if anyone_said(body, "has finished") {
            Script::Say("The coding agent fixed the flaky test.".into())
        } else if has_tool_result(body) {
            Script::Say("Started it. Checking back on it shortly.".into())
        } else {
            Script::Code("fix the flaky test".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());

    let repo = give_terminal(&h, "Engineer", repo, Which::Claude, Gate::Open);

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;

    assert!(
        h.channel_texts("Engineer").iter().any(|line| line.contains("Started it")),
        "the announcement is the answer to that turn: {:?}",
        h.channel_texts("Engineer")
    );
    assert!(
        !stub
            .transcript
            .lock()
            .iter()
            .any(|body| anyone_said(body, "You ended your message with work you had not done")),
        "a job that is genuinely running backs the sentence that announces it"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// A job is told where the tree is standing before it is told what to do.
///
/// This is the other seam nothing else in the repo can see. Drop the
/// `repo::footing` read in `Runtime::start_job` and every suite still passes,
/// while every job in every workspace goes on starting wherever the last one
/// left the tree: a branch that was merged a month ago, silently, on top of
/// work that has already landed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_is_told_which_branch_it_is_standing_on() {
    stand_ins();
    let repo = a_repository_on_a_landed_branch("footing");

    // A completed harness can release the repository before its completion
    // message reaches the model. Do not start another job in that interval:
    // it would overwrite the captured first brief after .argv dirtied the tree.
    let kickoff = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let stub = serve(move |body| {
        if anyone_said(body, "has finished") {
            Script::Say("The coding agent did the work.".into())
        } else if kickoff.swap(false, std::sync::atomic::Ordering::SeqCst) {
            Script::Code("fix the flaky test".into())
        } else {
            Script::Say("I have started it.".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());

    let repo = give_terminal(&h, "Engineer", repo, Which::Pi, Gate::Open);

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the coding job is reported back", |h| {
        h.channel_texts("Engineer").iter().any(|line| line.contains("did the work"))
    })
    .await;

    let brief = brief_seen(Which::Pi, &repo);
    assert!(brief.contains("fix the flaky test"), "the brief never reached the program");

    // The state, the rule it resolves to, then the work, in that order. The
    // footing leads because it is read before the first edit or it is not read
    // at all.
    assert!(brief.contains("On branch `landed`"), "{brief}");
    assert!(brief.contains("already contained in `main`"), "{brief}");
    assert!(brief.contains("start from `main`"), "{brief}");
    let state = brief.find("Where you are starting from").expect("no footing: {brief}");
    let work = brief.find("fix the flaky test").unwrap();
    assert!(state < work, "the footing comes after the work: {brief}");

    let _ = std::fs::remove_dir_all(&repo);
}

// ---- the other door ------------------------------------------------------

/// The small door, end to end: a real shell, in the agent's own terminal,
/// answering inside the turn that asked.
///
/// This is the seam nothing else can see. `shell` is offered on the same
/// condition as `code` and reads the same column, and the failure it exists to
/// stop is not a crash: it is an agent that has to spend a coding job, minutes
/// and somebody's plan on `git status`, and that reports having no shell at all
/// when the harness will not start.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_with_a_terminal_runs_a_line_there_and_is_answered_in_the_same_turn() {
    // Branched on the tool result rather than on a counter, because a turn can
    // take more than one call.
    let stub = serve(move |body| {
        if has_tool_result(body) {
            Script::Say("I am standing in my terminal.".into())
        } else {
            Script::Shell("pwd -P".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let home = give_terminal(&h, "Engineer", a_repository("shell-here"), Which::Claude, Gate::Open);

    let run = h.runtime.send_from_human(h.id("Engineer"), "which directory are you in?").unwrap();
    h.settle(run).await;

    // The needle is the terminal's own path, which is the whole assertion: a
    // shell that ran somewhere else answers with somewhere else.
    let told = tool_results(&stub).join("\n");
    assert!(
        told.contains(&home.to_string_lossy().to_string()),
        "the line did not run in the terminal:\n{told}"
    );
    assert!(
        h.channel_texts("Engineer").iter().any(|t| t.contains("standing in my terminal")),
        "and the turn finished on it:\n{}",
        h.transcript()
    );
}

/// An agent never given a terminal is told so, in words it can act on,
/// whichever tool it reached for.
///
/// None of them is offered to it, and a model names tools it was never offered
/// anyway. What it must not get is a shell by accident, or a refusal that reads
/// like something broke.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_without_a_terminal_is_told_who_can_give_it_one() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("I have no terminal.".into())
        } else {
            Script::Shell("touch was-here.txt".into())
        }
    })
    .await;
    let h = harness(&stub, &["Writer"], GuardLimits::default());
    let run = h.runtime.send_from_human(h.id("Writer"), "make a file").unwrap();
    h.settle(run).await;

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("has not been given a terminal"), "{told}");
    assert!(told.contains("operator"), "a refusal needs a way forward: {told}");
    let home = h.runtime.terminals().dir(h.id("Writer"));
    assert!(!home.exists(), "nothing was made for an agent that was never given one");
}

/// The three file tools and the shell are one directory.
///
/// What `write` made, `edit` changes and `read` reads back, and the shell sees
/// the same bytes. An edit that does not match changes nothing and says so
/// before anything else, because an edit a model believes landed is the next
/// edit's wrong `old_text`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_file_tools_and_the_shell_open_on_the_same_directory() {
    let stub = serve(|body| {
        let acted = body["messages"]
            .as_array()
            .map(|messages| messages.iter().filter(|m| m["role"] == "tool").count())
            .unwrap_or(0);
        match acted {
            0 => Script::Tool {
                name: "write".into(),
                arguments: serde_json::json!({"path": "site/config.toml", "content": "port = 80\nname = \"site\"\n"}),
            },
            1 => Script::Tool {
                name: "edit".into(),
                arguments: serde_json::json!({"path": "site/config.toml", "edits": [{"old_text": "port = 80", "new_text": "port = 8080"}]}),
            },
            2 => Script::Tool {
                name: "edit".into(),
                arguments: serde_json::json!({"path": "site/config.toml", "edits": [{"old_text": "port = 9999", "new_text": "port = 1"}]}),
            },
            3 => Script::Tool {
                name: "read".into(),
                arguments: serde_json::json!({"path": "site/config.toml"}),
            },
            4 => Script::Shell("cat site/config.toml".into()),
            _ => Script::Say("The port is 8080.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let home = give_terminal(&h, "Engineer", a_repository("file-tools"), Which::Pi, Gate::Open);

    let run = h.runtime.send_from_human(h.id("Engineer"), "move the site to 8080").unwrap();
    h.settle(run).await;

    let told = last_tool_results(&stub);
    assert!(told[0].contains("Created site/config.toml"), "{told:?}");
    assert!(told[1].contains("Edited site/config.toml: 1 replacement"), "{told:?}");
    assert!(told[2].contains("not in the file"), "{told:?}");
    assert!(told[2].contains("The file was not changed"), "{told:?}");
    assert!(told[3].contains("port = 8080") && told[3].contains("lines 1-2 of 2"), "{told:?}");
    assert!(told[4].contains("port = 8080"), "the shell reads another directory: {told:?}");
    assert_eq!(
        std::fs::read_to_string(home.join("site/config.toml")).unwrap(),
        "port = 8080\nname = \"site\"\n"
    );
}

/// The gate is a fact about the agent, so it cannot mean one thing through
/// `code` and another through `shell`.
///
/// Both doors ask `coding::bridge::outward` about the same shell line, from the
/// same function. A gate that read only the harness's calls would be a gate an
/// agent walks around by picking the other tool, which is worse than no gate:
/// the operator switched it on and would be told it was holding.
///
/// The line is deliberately two commands. A `deny` refuses the *call*, exactly
/// as the `PreToolUse` hook does, so the harmless half must not have happened
/// either — a refusal that ran the first half and stopped at the push is a
/// tree in a state nobody asked for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_line_that_reaches_outside_a_gated_repository_asks_first_and_a_no_runs_nothing() {
    let repo = a_repository("shell-gated");

    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("The operator did not allow the push.".into())
        } else {
            Script::Shell("touch pushed.txt && git push origin main".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let repo = give_terminal(&h, "Engineer", repo, Which::Claude, Gate::AskBeforePushing);

    let run = h.runtime.send_from_human(h.id("Engineer"), "ship it").unwrap();

    let request = h.awaited_request().await;
    h.runtime.decide_approval(request, Decision::Deny).unwrap();
    h.settle(run).await;

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("Refused"), "the model was not told it was refused:\n{told}");
    assert!(told.contains("waiting on them"), "a refusal needs a way forward:\n{told}");
    assert!(
        !repo.join("pushed.txt").exists(),
        "the call was refused, so no part of the line may have run"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// And the gate stops nothing else.
///
/// Everything that is not outward-facing is what the directory and git already
/// cover, in both doors. A gate that parked `git status` would be one the
/// operator switches off within the hour, which is the behavior they turned it
/// on to get.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ordinary_line_in_a_gated_repository_runs_without_asking_anybody() {
    let repo = a_repository("shell-ungated");

    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("Nothing is staged.".into())
        } else {
            Script::Shell("git status --porcelain; echo read-the-tree".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let repo = give_terminal(&h, "Engineer", repo, Which::Claude, Gate::AskBeforePushing);

    let run = h.runtime.send_from_human(h.id("Engineer"), "anything uncommitted?").unwrap();
    h.settle(run).await;

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("read-the-tree"), "the line did not run:\n{told}");
    assert!(
        h.runtime.store().pending_approvals(10).unwrap().is_empty(),
        "nobody should have been asked about reading the tree"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// The gate reads what the line runs, not only what it says.
///
/// `./scripts/ship.sh` is not `git push` and no amount of reading the words
/// would ever make it one. So a repository whose release lives in a script had
/// a gate that was switched on, said it was holding, and stopped nothing —
/// which is worse than no gate, because the operator was told it was working.
///
/// The card has to carry both halves. `./scripts/ship.sh` is what the agent
/// asked for and says nothing about what it does; `git push` is what the
/// operator is actually being asked to allow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_push_kept_in_one_of_the_operators_own_scripts_still_asks_first() {
    let repo = a_repository("shell-scripted");
    std::fs::create_dir_all(repo.join("scripts")).unwrap();
    std::fs::write(
        repo.join("scripts/ship.sh"),
        "#!/bin/sh\nset -e\ntouch shipped.txt\ngit push origin main\n",
    )
    .unwrap();

    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("The operator did not allow the release.".into())
        } else {
            Script::Shell("./scripts/ship.sh".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let repo = give_terminal(&h, "Engineer", repo, Which::Claude, Gate::AskBeforePushing);

    let run = h.runtime.send_from_human(h.id("Engineer"), "cut the release").unwrap();

    let request = h.awaited_request().await;
    let card = h
        .runtime
        .store()
        .pending_approvals(10)
        .unwrap()
        .into_iter()
        .find(|approval| approval.id == request)
        .expect("the request the operator is looking at");
    assert!(card.summary.contains("./scripts/ship.sh"), "{}", card.summary);
    assert!(card.summary.contains("git push"), "{}", card.summary);
    // The label says Command, so the field holds the line rather than what
    // this made of it.
    assert_eq!(
        card.detail.iter().find(|field| field.label == "Command").map(|field| &field.value),
        Some(&"./scripts/ship.sh".to_string()),
        "{:?}",
        card.detail
    );

    h.runtime.decide_approval(request, Decision::Deny).unwrap();
    h.settle(run).await;

    assert!(
        !repo.join("shipped.txt").exists(),
        "the call was refused, so no part of the script may have run"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// One no settles the question for the rest of the run.
///
/// A model that has just been refused a push tries the push. That is ordinary
/// rather than confused: what it reads back says the operator did not allow it,
/// not that they never will. The operator is the one who pays for it, in a
/// second card and a third, for a question they are sitting there answering.
///
/// The second line is deliberately spelled differently. What was refused is the
/// push, and a memory that told `git push origin main` from `git push --force`
/// would remember nothing a retry could not walk around.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refusal_is_not_put_to_the_operator_twice_in_one_run() {
    let repo = a_repository("shell-refused-twice");

    let stub = serve(|body| {
        let acted = body["messages"]
            .as_array()
            .map(|messages| messages.iter().filter(|m| m["role"] == "tool").count())
            .unwrap_or(0);
        match acted {
            0 => Script::Shell("git push origin main".into()),
            1 => Script::Shell("git push --force-with-lease origin main".into()),
            _ => Script::Say("Both attempts were refused.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let repo = give_terminal(&h, "Engineer", repo, Which::Claude, Gate::AskBeforePushing);

    let run = h.runtime.send_from_human(h.id("Engineer"), "push it").unwrap();

    let request = h.awaited_request().await;
    h.runtime.decide_approval(request, Decision::Deny).unwrap();
    h.settle(run).await;

    assert_eq!(
        h.sink.count_of(|event| matches!(event, UiEvent::ApprovalRequested { .. })),
        1,
        "the operator answered this once and was asked once"
    );
    // And the second attempt was still refused, rather than quietly allowed by
    // a gate that had stopped asking. Every result the model was handed is a
    // refusal, so neither line reached the shell.
    let told = tool_results(&stub);
    assert!(told.len() >= 2, "both lines have to have been tried: {told:?}");
    assert!(told.iter().all(|result| result.contains("Refused")), "{told:?}");
    let _ = std::fs::remove_dir_all(&repo);
}

/// The gate follows a `cd` into a repository, which is how a line in a
/// terminal holding several of them reaches one.
///
/// Read from the top of the terminal, `cd site && ./scripts/ship.sh` names a
/// script that is not there, and a gate that stopped reading at that point was
/// switched on and held nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_push_in_a_script_one_cd_down_still_asks_first() {
    let stub = serve(|body| {
        if has_tool_result(body) {
            Script::Say("The operator did not allow the release.".into())
        } else {
            Script::Shell("cd site && ./scripts/ship.sh".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let home = give_terminal(
        &h,
        "Engineer",
        a_repository("shell-cd-script"),
        Which::Claude,
        Gate::AskBeforePushing,
    );
    std::fs::create_dir_all(home.join("site/scripts")).unwrap();
    std::fs::write(
        home.join("site/scripts/ship.sh"),
        "#!/bin/sh\nset -e\ntouch shipped.txt\ngit push origin main\n",
    )
    .unwrap();

    let run = h.runtime.send_from_human(h.id("Engineer"), "cut the release").unwrap();
    let request = h.awaited_request().await;
    let card = h
        .runtime
        .store()
        .pending_approvals(10)
        .unwrap()
        .into_iter()
        .find(|approval| approval.id == request)
        .expect("the request the operator is looking at");
    assert!(card.summary.contains("git push"), "{}", card.summary);
    assert!(card.summary.contains("./scripts/ship.sh"), "{}", card.summary);
    h.runtime.decide_approval(request, Decision::Deny).unwrap();
    h.settle(run).await;
    assert!(!home.join("site/shipped.txt").exists(), "no part of the refused line may run");
}

/// The door that stays open when the other one will not.
///
/// An agent with a job already running is refused `code`, on purpose: two
/// harnesses in one directory interleave their edits. One line is not that, and
/// refusing it here would take away the read an agent most wants while a job
/// runs, which is what the job is doing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_line_still_runs_while_the_agents_coding_job_is_going() {
    stand_ins();
    let repo = a_repository("shell-alongside");
    // Slow enough that the job is genuinely still running when the line does.
    std::fs::write(repo.join(LINGER), "3").unwrap();

    let stub = serve(|body| {
        if anyone_said(body, "alongside-the-job") {
            Script::Say("The job is still going.".into())
        } else if has_tool_result(body) {
            Script::Shell("echo alongside-the-job".into())
        } else {
            Script::Code("something long".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let repo = give_terminal(&h, "Engineer", repo, Which::Claude, Gate::Open);

    let run =
        h.runtime.send_from_human(h.id("Engineer"), "start it and tell me where we are").unwrap();
    h.settle(run).await;

    let told = tool_results(&stub).join("\n");
    assert!(told.contains("A coding agent is working"), "the job did not start:\n{told}");
    assert!(told.contains("alongside-the-job"), "the line was refused or never ran:\n{told}");
    let _ = std::fs::remove_dir_all(&repo);
}

// ---- the half no offline test can see ------------------------------------

/// Whether the real `claude` still accepts the vector this build sends.
///
/// Everything above is this app agreeing with itself about a protocol. The
/// failure worth catching is that belief going stale: a flag renamed, a mode
/// that now needs another flag, a stream whose events changed shape. It makes
/// one real model call against the operator's own Claude sign-in.
#[tokio::test]
#[ignore = "live: spends the operator's own Claude plan"]
async fn the_real_claude_still_answers_the_way_this_build_reads() {
    let repo = a_repository("live-claude");
    std::fs::write(repo.join("a.txt"), "banana").unwrap();

    let outcome = run_job(
        Which::Claude,
        &repo,
        "Read a.txt and say what one word it contains. Change nothing and commit nothing.",
        |_| {},
    )
    .await
    .expect("the harness has to start and answer");

    assert_eq!(outcome.failed, None, "the sign-in is spent or the vector is stale");
    assert!(outcome.said.to_lowercase().contains("banana"), "{}", outcome.said);
    assert!(outcome.tool_calls > 0, "it has to have read the file rather than guessed");
    assert!(!outcome.model.is_empty(), "the stream still names the model");
    let _ = std::fs::remove_dir_all(&repo);
}

/// A job can be ended, and what it committed is not taken back with it.
///
/// The gap this closes is that there was no way to end one at all. A job runs
/// for up to forty-five minutes, `code` returns the moment the process is up,
/// and stopping the conversation that started it does not touch the job:
/// that run settled minutes earlier. The ceiling was the only thing that ever
/// ended one that was going wrong.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_going_the_wrong_way_can_be_stopped_and_the_agent_is_told() {
    stand_ins();
    for which in Which::ALL {
        let repo = a_repository(&format!("stopped-{}", which.as_str()));
        // Long enough that the test reaches it while it is still running, and
        // short enough that a broken stop fails the test rather than hanging it.
        // Codex's stand-in holds its turn open instead, until it is steered or
        // interrupted.
        std::fs::write(repo.join(LINGER), "30").unwrap();
        std::fs::write(repo.join(".codex_hold"), "").unwrap();

        let stub = serve(|body| {
            if anyone_said(body, "stopped the coding agent") {
                Script::Say("I have stopped it.".into())
            } else {
                Script::Code("fix the flaky test".into())
            }
        })
        .await;
        let h = harness(&stub, &["Engineer"], GuardLimits::default());
        let engineer = h.agent_named("Engineer").unwrap();
        let repo = give_terminal(&h, "Engineer", repo, which, Gate::Open);

        let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
        h.settle(run).await;
        // Codex has a turn to interrupt only once `turn/start` is answered.
        h.wait_until("the harness is working", |_| match which {
            Which::Codex => brief_seen(which, &repo).contains("turn/start"),
            Which::Claude | Which::Pi => !brief_seen(which, &repo).is_empty(),
        })
        .await;

        h.runtime
            .stop_job(engineer.id, Origin::Operator)
            .expect("a running job has to be stoppable");

        // Told, rather than left waiting for a message that is not coming. An
        // agent that is never told answers "I started that and have not heard
        // back", which is true and useless.
        h.wait_until("the agent is told it was stopped", |h| {
            h.channel_texts("Engineer").iter().any(|line| line.contains("stopped the coding agent"))
        })
        .await;
        // Each program stopped the way its own interface stops it, so the
        // stop is something the program recorded rather than a process that
        // vanished mid-message.
        match which {
            Which::Codex => assert!(repo.join(".interrupted").exists(), "turn/interrupt"),
            Which::Pi => assert!(repo.join(".aborted").exists(), "abort"),
            Which::Claude => assert!(repo.join(".interrupted").exists(), "the SDK's interrupt"),
        }

        // And the lane is free, so the next brief does not come back busy about
        // a job that is over.
        h.runtime
            .stop_job(engineer.id, Origin::Operator)
            .expect_err("a stopped job is not a running one");
        let _ = std::fs::remove_dir_all(&repo);
    }
}

/// A stop the agent asked for is not news to the agent.
///
/// The operator's stop is: the agent started that job and is waiting on it.
/// Its own `code` `stop` is a decision it has just taken, and telling it back
/// is a turn spent reading its own words.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_the_agent_asked_for_is_not_reported_back_to_it() {
    stand_ins();
    let repo = a_repository("stopped-by-agent");
    std::fs::write(repo.join(LINGER), "30").unwrap();
    let stub = serve(|body| {
        if anyone_said(body, "stopped the coding agent") {
            Script::Say("told".into())
        } else {
            Script::Code("fix the flaky test".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let engineer = h.agent_named("Engineer").unwrap();
    let repo = give_terminal(&h, "Engineer", repo, Which::Pi, Gate::Open);

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the harness is working", |_| repo.join(".pi_prompt").exists()).await;
    h.runtime.stop_job(engineer.id, Origin::Agent).unwrap();
    h.wait_until("the lane is free", |h| {
        h.runtime.stop_job(engineer.id, Origin::Operator).is_err()
    })
    .await;
    assert!(repo.join(".aborted").exists());
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !h.channel_texts("Engineer").iter().any(|line| line.contains("stopped the coding agent")),
        "{:?}",
        h.channel_texts("Engineer")
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// Pressing stop twice is not an error, and neither is pressing it late.
///
/// Both are the ordinary case rather than a confused caller: a job that has
/// been running for forty minutes is one an operator presses a button on at
/// exactly the moment it ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_a_job_that_is_already_over_says_so_rather_than_failing() {
    let h = harness(
        &serve(|_| Script::Say("hello".into())).await,
        &["Engineer"],
        GuardLimits::default(),
    );
    let engineer = h.agent_named("Engineer").unwrap();
    let _repo = give_terminal(&h, "Engineer", a_repository("stop-twice"), Which::Pi, Gate::Open);

    let why = h.runtime.stop_job(engineer.id, Origin::Operator).unwrap_err().to_string();
    assert!(why.contains("already finished"), "{why}");
}

/// A correction typed into a running pi job reaches the same process.
///
/// pi's RPC mode is the interface its own editor uses to steer a turn, which is
/// what Guaca has to match: the correction lands between tool calls, in the
/// turn that is running, rather than queued behind it as a second job.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_operator_can_steer_pi_while_it_works() {
    stand_ins();
    let repo = a_repository("pi-steered");
    std::fs::write(repo.join(LINGER), "30").unwrap();

    let stub = serve(|body| {
        if anyone_said(body, "has finished") || anyone_said(body, "stopped the coding agent") {
            Script::Say("done".into())
        } else {
            Script::Code("fix the flaky test".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let engineer = h.agent_named("Engineer").unwrap();
    let repo = give_terminal(&h, "Engineer", repo, Which::Pi, Gate::Open);

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the harness is working", |_| repo.join(".pi_prompt").exists()).await;

    let continued = h.runtime.message_job(engineer.id, "use the other endpoint").await.unwrap();
    assert!(matches!(continued, Continued::Steered), "{continued:?}");
    h.wait_until("the correction is read", |_| repo.join(".steered").exists()).await;
    assert_eq!(std::fs::read_to_string(repo.join(".steered")).unwrap(), "use the other endpoint");
    assert_eq!(
        std::fs::read_to_string(repo.join(".pi_history")).unwrap().lines().count(),
        1,
        "a correction is not a second prompt"
    );

    h.runtime.stop_job(engineer.id, Origin::Operator).unwrap();
    let _ = std::fs::remove_dir_all(&repo);
}

/// A finished job can be carried on, in its own session, by whoever asks.
///
/// Every one of the three programs keeps a session and can be handed it back:
/// `claude --resume`, Codex's `thread/resume`, pi's `--session-id` on an id it
/// already holds. The follow-up has to arrive with everything the job already
/// read, which is the whole difference between continuing and starting over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finished_job_is_continued_in_the_session_it_ran_in() {
    stand_ins();
    for which in Which::ALL {
        let repo = a_repository(&format!("continued-{}", which.as_str()));
        let stub = serve(|body| {
            if anyone_said(body, "has finished") {
                Script::Say("reported".into())
            } else {
                Script::Code("fix the flaky test".into())
            }
        })
        .await;
        let h = harness(&stub, &["Engineer"], GuardLimits::default());
        let engineer = h.agent_named("Engineer").unwrap();
        let repo = give_terminal(&h, "Engineer", repo, which, Gate::Open);

        let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
        h.settle(run).await;
        h.wait_until("the first job is reported", |h| {
            h.channel_texts("Engineer").iter().any(|line| line.contains("has finished"))
        })
        .await;
        let first = argv_at(&repo);
        let session = h.runtime.store().coding_session(engineer.id).unwrap().expect("kept");
        assert_eq!(session.harness, which);
        assert_eq!(session.directory, ".");

        let continued = h.runtime.message_job(engineer.id, "now add a test for it").await.unwrap();
        assert!(
            matches!(&continued, Continued::Resumed { directory } if directory == "."),
            "{continued:?}"
        );
        // The agent that started the job is told the operator went round it,
        // because it is the one that will be asked what came of it.
        h.wait_until("the follow-up is reported", |h| {
            h.channel_texts("Engineer").iter().any(|line| line.contains("follow-up directly"))
        })
        .await;
        assert!(brief_seen(which, &repo).contains("The operator says: now add a test for it"));

        match which {
            Which::Claude => {
                let argv = argv_at(&repo);
                let after = |flag: &str, argv: &[String]| {
                    argv.iter().position(|arg| arg == flag).map(|at| argv[at + 1].clone())
                };
                assert_eq!(after("--session-id", &first), Some(session.id.clone()));
                assert_eq!(after("--resume", &argv), Some(session.id.clone()), "{argv:?}");
            }
            Which::Codex => {
                assert_eq!(session.id, "codex-session", "the thread the program named");
                assert_eq!(
                    std::fs::read_to_string(repo.join(".resumed")).unwrap(),
                    "codex-session"
                );
            }
            Which::Pi => {
                let history = std::fs::read_to_string(repo.join(".pi_history")).unwrap();
                let sessions: Vec<&str> =
                    history.lines().map(|line| line.split(' ').next().unwrap()).collect();
                assert_eq!(sessions, [session.id.as_str(), session.id.as_str()], "{history}");
            }
        }
        let _ = std::fs::remove_dir_all(&repo);
    }
}

/// A session is carried on by the program that wrote it, or not at all.
///
/// Switching an agent from Claude Code to Codex because a plan ran out is the
/// ordinary case, and Codex has never heard of the session id Claude Code
/// minted. The refusal names both, so the way on is a fresh `start`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_is_not_continued_by_a_different_harness() {
    stand_ins();
    let repo = a_repository("continued-elsewhere");
    let stub = serve(|body| {
        if anyone_said(body, "has finished") {
            Script::Say("reported".into())
        } else {
            Script::Code("fix the flaky test".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let engineer = h.agent_named("Engineer").unwrap();
    let repo = give_terminal(&h, "Engineer", repo, Which::Claude, Gate::Open);

    let why = h.runtime.message_job(engineer.id, "carry on").await.unwrap_err().to_string();
    assert!(why.contains("start"), "nothing has run yet: {why}");

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the first job is reported", |h| {
        h.channel_texts("Engineer").iter().any(|line| line.contains("has finished"))
    })
    .await;
    h.runtime.store().set_agent_coding(engineer.id, Which::Codex, Gate::Open).unwrap();

    let why = h.runtime.message_job(engineer.id, "carry on").await.unwrap_err().to_string();
    assert!(why.contains("Claude Code") && why.contains("Codex"), "{why}");
    assert!(!repo.join(".rpc.jsonl").exists(), "Codex was never started on it");
    let _ = std::fs::remove_dir_all(&repo);
}

/// pi's gate is answered by the operator, both ways, over the same RPC.
///
/// The extension decides nothing: it asks `confirm` about every `bash` call and
/// Guaca reads the line. A no has to reach the program as a no, or the command
/// runs anyway.
#[tokio::test]
async fn the_pi_gate_runs_a_push_only_when_the_operator_says_so() {
    stand_ins();
    for allow in [true, false] {
        let repo = a_repository(&format!("pi-gate-{allow}"));
        let path = repo.to_string_lossy().to_string();
        let (_controls, controlled) = tokio::sync::mpsc::channel(8);
        let (signals, mut heard) = tokio::sync::mpsc::channel(8);
        let job = tokio::spawn(async move {
            let env = guac_lib::secrets::Environment::default();
            coding::run(
                job(Which::Pi, &path, "work", Gate::AskBeforePushing, &env),
                controlled,
                signals,
                |_| {},
            )
            .await
        });
        let signal = tokio::time::timeout(std::time::Duration::from_secs(5), heard.recv())
            .await
            .unwrap()
            .unwrap();
        let coding::Signal::Permission { line, reply, .. } = signal else { panic!("wrong signal") };
        assert_eq!(line, "git push origin HEAD");
        assert!(!repo.join(".pushed").exists(), "nothing runs before the answer");
        reply.send(allow).unwrap();
        job.await.unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join(".verdict")).unwrap(),
            if allow { "confirmed" } else { "declined" }
        );
        assert_eq!(repo.join(".pushed").exists(), allow);
        let _ = std::fs::remove_dir_all(repo);
    }
}

/// What the operator chose inside a harness reaches the program as the
/// program's own flag, through the runtime, for each of the three.
///
/// Read from the store when the job starts, per harness, so the Claude model
/// an agent had is still its Claude model after an afternoon on Codex.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chosen_model_and_effort_reach_each_program_as_its_own_flag() {
    stand_ins();
    for which in Which::ALL {
        let repo = a_repository(&format!("tuned-{}", which.as_str()));
        let stub = serve(|body| match anyone_said(body, "has finished") {
            true => Script::Say("reported".into()),
            false => Script::Code("fix the flaky test".into()),
        })
        .await;
        let h = harness(&stub, &["Engineer"], GuardLimits::default());
        let engineer = h.agent_named("Engineer").unwrap();
        let repo = give_terminal(&h, "Engineer", repo, which, Gate::Open);
        let (model, effort) = match which {
            Which::Claude => ("opus", "max"),
            Which::Codex => ("gpt-5.5", "high"),
            Which::Pi => ("anthropic/claude-opus-4-7", "high"),
        };
        let tuning =
            Tuning { model: Some(model.into()), effort: Some(effort.into()), ..Tuning::default() };
        h.runtime
            .store()
            .set_coding_tuning(engineer.id, which, &tuning.clean(which).unwrap())
            .unwrap();

        let run = h.runtime.send_from_human(engineer.id, "fix the flaky test").unwrap();
        h.settle(run).await;
        h.wait_until("the job is reported", |h| {
            h.channel_texts("Engineer").iter().any(|line| line.contains("has finished"))
        })
        .await;

        let argv = argv_at(&repo);
        let after =
            |flag: &str| argv.iter().position(|arg| arg == flag).map(|at| argv[at + 1].clone());
        match which {
            Which::Claude => {
                assert_eq!(after("--model").as_deref(), Some(model));
                assert_eq!(after("--effort").as_deref(), Some(effort));
            }
            Which::Codex => {
                let log = std::fs::read_to_string(repo.join(".rpc.jsonl")).unwrap();
                let sent: Vec<serde_json::Value> =
                    log.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
                let method =
                    |name: &str| sent.iter().find(|m| m["method"] == name).cloned().unwrap();
                assert_eq!(method("thread/start")["params"]["model"], model);
                assert_eq!(method("turn/start")["params"]["effort"], effort);
            }
            Which::Pi => {
                assert_eq!(after("--model").as_deref(), Some(model));
                assert_eq!(after("--thinking").as_deref(), Some(effort));
                assert!(!argv.contains(&"--provider".to_string()), "pi's own sign-in pays");
            }
        }
        let _ = std::fs::remove_dir_all(&repo);
    }
}

/// pi on Guaca's key is paid for through the relay, and never holds the key.
///
/// The stand-in calls the provider its extension names, as pi does, so this
/// watches the whole path: the extension, the token, the relay, the endpoint
/// in settings, and the answer streamed back. Then the job ends and the token
/// is worth nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pi_on_guacas_key_pays_through_the_relay_and_never_holds_the_key() {
    stand_ins();
    let repo = a_repository("lent");
    let stub = serve(|body| {
        if anyone_said(body, "relay probe") {
            Script::Say("relayed answer".into())
        } else if anyone_said(body, "has finished") {
            Script::Say("reported".into())
        } else {
            Script::Code("fix the flaky test".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let engineer = h.agent_named("Engineer").unwrap();
    let repo = give_terminal(&h, "Engineer", repo, Which::Pi, Gate::Open);
    let lent = Tuning { pays: Payer::GuacaKey, ..Tuning::default() };
    h.runtime.store().set_coding_tuning(engineer.id, Which::Pi, &lent).unwrap();

    let run = h.runtime.send_from_human(engineer.id, "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the job is reported", |h| {
        h.channel_texts("Engineer").iter().any(|line| line.contains("has finished"))
    })
    .await;

    // The endpoint in settings is not OpenRouter, so the job gets a provider
    // of one model, the key's own, since none was chosen.
    let argv = argv_at(&repo);
    let after = |flag: &str| argv.iter().position(|arg| arg == flag).map(|at| argv[at + 1].clone());
    assert_eq!(after("--provider").as_deref(), Some("guaca"));
    assert_eq!(after("--model").as_deref(), Some("test/model"));
    // The call went all the way through and came back.
    let relayed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.join(".relayed")).unwrap()).unwrap();
    assert_eq!(relayed["status"], 200, "{relayed}");
    let streamed = relayed["body"].as_str().unwrap();
    assert!(
        streamed.contains(r#""content":"relayed""#) && streamed.contains("[DONE]"),
        "{relayed}"
    );
    // Metered as it passed: the call is on the bill, against the job's own
    // run rather than any turn's, the way a turn's own call is.
    let turns: std::collections::HashSet<_> = h
        .runtime
        .store()
        .channel_messages(engineer.id, 200)
        .unwrap()
        .into_iter()
        .map(|message| message.run_id)
        .collect();
    let metered = h.sink.snapshot().into_iter().any(|event| {
        matches!(event, UiEvent::TokensUsed { agent_id, run_id, prompt: 100, completion: 20, .. }
            if agent_id == engineer.id && !turns.contains(&run_id))
    });
    assert!(metered, "the relayed call reached the tally");

    // And what pi was handed is a loopback address and a token, not the key.
    let handed = std::fs::read_to_string(repo.join(".extensions")).unwrap();
    assert!(!handed.contains("sk-test"), "{handed}");
    assert!(handed.contains("http://127.0.0.1:"), "{handed}");

    // Over now, so its token is refused.
    let token = regex_lite_token(&handed);
    let base = handed.split("baseUrl: \"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let answer = reqwest::Client::new()
        .post(format!("{base}/chat/completions"))
        .bearer_auth(token)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), 401);
    let _ = std::fs::remove_dir_all(&repo);
}

/// The token a provider extension carries, out of its source.
fn regex_lite_token(source: &str) -> String {
    source.split("apiKey: \"").nth(1).unwrap().split('"').next().unwrap().to_string()
}

/// A job set to Guaca's key, on a workspace with no key, never starts.
///
/// Refused where it was asked for, with the way on, rather than minutes later
/// as a job pi could not pay for. Asked through a follow-up, because the turn
/// that would call `code` is paid for with the same key and could not run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_on_guacas_key_with_no_key_is_refused_before_anything_starts() {
    stand_ins();
    let stub = serve(|_| Script::Say("hello".into())).await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let engineer = h.agent_named("Engineer").unwrap();
    let repo = give_terminal(&h, "Engineer", a_repository("lent-nothing"), Which::Pi, Gate::Open);
    let lent = Tuning { pays: Payer::GuacaKey, ..Tuning::default() };
    h.runtime.store().set_coding_tuning(engineer.id, Which::Pi, &lent).unwrap();
    let session = guac_lib::domain::terminal::Session {
        harness: Which::Pi,
        id: "s1".into(),
        directory: ".".into(),
        updated_at: 1,
    };
    h.runtime.store().set_coding_session(engineer.id, &session).unwrap();
    let mut config = h.runtime.config();
    config.inference.api_key = String::new();
    h.runtime.set_config(config);

    let why = h.runtime.message_job(engineer.id, "carry on").await.unwrap_err().to_string();
    assert!(why.contains("no key in your group's settings or in Settings > Provider"), "{why}");
    // And the two ways on, because a refusal that only says no is retried.
    assert!(why.contains("paste a key") && why.contains("its own sign-in"), "{why}");
    assert!(!repo.join(ARGV).exists(), "pi was never started");
    assert!(h.runtime.stop_job(engineer.id, Origin::Operator).is_err(), "nothing holds the lane");
    let _ = std::fs::remove_dir_all(&repo);
}

/// A crew whose turns a ChatGPT sign-in pays for lends pi the key it holds.
///
/// The operator's case: the crew's settings hold an OpenRouter key and pay
/// for turns with the subscription, the app's hold no key at all, and pi is
/// set to Guaca's key. Reading the app's settings alone refused the job. The
/// app's endpoint here answers nothing, so a relayed answer can only have come
/// from the crew's, on the crew's key, running the crew's endpoint model
/// rather than the subscription's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crew_on_a_subscription_lends_pi_the_key_it_holds() {
    use guac_lib::config::Provider;
    use guac_lib::domain::group::{CleanGroup, InferenceOverrides};

    stand_ins();
    let stub = serve(|body| {
        if anyone_said(body, "relay probe") {
            Script::Say("relayed answer".into())
        } else {
            Script::Say("hello".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let engineer = h.agent_named("Engineer").unwrap();
    let repo = give_terminal(&h, "Engineer", a_repository("lent-crew"), Which::Pi, Gate::Open);
    let lent = Tuning { pays: Payer::GuacaKey, ..Tuning::default() };
    h.runtime.store().set_coding_tuning(engineer.id, Which::Pi, &lent).unwrap();
    let session = guac_lib::domain::terminal::Session {
        harness: Which::Pi,
        id: "s1".into(),
        directory: ".".into(),
        updated_at: 1,
    };
    h.runtime.store().set_coding_session(engineer.id, &session).unwrap();
    let mut config = h.runtime.config();
    config.inference.api_key = String::new();
    config.inference.base_url = "http://127.0.0.1:9/v1".into();
    h.runtime.set_config(config);
    h.runtime
        .store()
        .update_group(
            engineer.group_id,
            &CleanGroup {
                name: "SynopsisMD".into(),
                inference: Some(InferenceOverrides {
                    provider: Some(Provider::Chatgpt),
                    base_url: Some(stub.base_url.clone()),
                    default_model: Some("crew/model".into()),
                    subscription_model: Some("gpt-6-astra".into()),
                    ..Default::default()
                }),
                api_key: Some(Some("sk-crew".into())),
                limits: None,
            },
        )
        .unwrap();

    h.runtime.message_job(engineer.id, "carry on").await.unwrap();
    h.wait_until("the job calls its provider", |_| repo.join(".relayed").exists()).await;

    let argv = argv_at(&repo);
    let after = |flag: &str| argv.iter().position(|arg| arg == flag).map(|at| argv[at + 1].clone());
    assert_eq!(after("--provider").as_deref(), Some("guaca"));
    assert_eq!(after("--model").as_deref(), Some("crew/model"), "the endpoint's model");
    let relayed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.join(".relayed")).unwrap()).unwrap();
    assert_eq!(relayed["status"], 200, "{relayed}");
    let handed = std::fs::read_to_string(repo.join(".extensions")).unwrap();
    assert!(!handed.contains("sk-crew"), "{handed}");
    let _ = std::fs::remove_dir_all(&repo);
}

/// The relay puts the key on, takes the token off, and admits nothing else.
#[tokio::test]
async fn the_relay_lends_the_key_to_a_live_token_and_to_nothing_else() {
    use axum::{routing::post, Router};
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let recorder = seen.clone();
    let upstream = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: axum::http::HeaderMap| {
            let recorder = recorder.clone();
            async move {
                let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
                recorder.lock().push(auth.to_string());
                ([("content-type", "text/event-stream")], "data: {\"ok\":true}\n\ndata: [DONE]\n\n")
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });

    let relay = coding::relay::Relay::new();
    let lease = relay
        .lend(
            coding::relay::Upstream {
                base_url: format!("http://{address}/v1"),
                api_key: "sk-the-real-one".into(),
                referer: "https://example.com".into(),
                title: "Guac".into(),
            },
            None,
        )
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let post = |token: &str, path: &str| {
        client
            .post(format!("{}{path}", lease.base_url().trim_end_matches("/v1")))
            .bearer_auth(token.to_string())
            .body("{}")
    };

    let answer = post(lease.token(), "/v1/chat/completions").send().await.unwrap();
    assert_eq!(answer.status(), 200);
    assert_eq!(answer.headers()["content-type"], "text/event-stream");
    assert!(answer.text().await.unwrap().contains("[DONE]"), "streamed back whole");
    assert_eq!(*seen.lock(), ["Bearer sk-the-real-one"], "the key, never the token");

    assert_eq!(post("not-a-job", "/v1/chat/completions").send().await.unwrap().status(), 401);
    assert_eq!(post(lease.token(), "/v1/embeddings").send().await.unwrap().status(), 404);

    let token = lease.token().to_string();
    let base = lease.base_url();
    drop(lease);
    assert_eq!(relay.outstanding(), 0);
    let late = client
        .post(format!("{base}/chat/completions"))
        .bearer_auth(token)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(late.status(), 401, "a job's token ends with the job");
    assert_eq!(seen.lock().len(), 1, "nothing refused ever reached the endpoint");
}

/// Each harness lists its models the way its own picker would.
#[tokio::test]
async fn each_harness_lists_its_models_the_way_its_own_picker_would() {
    stand_ins();
    let claude = coding::models(Which::Claude, None).await.unwrap();
    let ids: Vec<&str> = claude.iter().map(|offer| offer.id.as_str()).collect();
    assert_eq!(ids, ["claude-fable-5-1", "haiku"]);
    assert!(claude[0].default);

    let codex = coding::models(Which::Codex, None).await.unwrap();
    assert_eq!(codex.len(), 1, "hidden models are not offered");
    assert_eq!(codex[0].efforts, ["low", "ultra"]);

    let own = coding::models(Which::Pi, None).await.unwrap();
    assert_eq!(own[0].id, "anthropic/claude-opus-4-7");
    let upstream = |base: &str| coding::relay::Upstream {
        base_url: base.into(),
        api_key: String::new(),
        referer: String::new(),
        title: String::new(),
    };
    // On Guaca's key, what that key can reach: OpenRouter's models only.
    let lent =
        coding::models(Which::Pi, Some(&upstream("https://openrouter.ai/api/v1"))).await.unwrap();
    let ids: Vec<&str> = lent.iter().map(|offer| offer.id.as_str()).collect();
    assert_eq!(ids, ["qwen/qwen3-coder"]);
    // Anywhere else there is no catalog to ask, and pi's own sign-ins would
    // offer models the job cannot reach.
    let elsewhere =
        coding::models(Which::Pi, Some(&upstream("http://127.0.0.1:1234/v1"))).await.unwrap();
    assert!(elsewhere.is_empty());
}

/// A gate that did not load is a job that does not start.
///
/// Failing open here would be a push the operator said to ask about, run
/// without asking, by a job that looked exactly like a gated one.
#[tokio::test]
async fn a_pi_job_whose_gate_did_not_load_is_refused_before_the_brief() {
    stand_ins();
    let repo = a_repository("pi-no-gate");
    std::fs::write(repo.join(".pi_no_gate"), "").unwrap();
    let path = repo.to_string_lossy().to_string();
    let (_controls, controlled) = tokio::sync::mpsc::channel(8);
    let (signals, _heard) = tokio::sync::mpsc::channel(8);
    let env = guac_lib::secrets::Environment::default();
    let why = coding::run(
        job(Which::Pi, &path, "work", Gate::AskBeforePushing, &env),
        controlled,
        signals,
        |_| {},
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(why.contains("did not load Guaca's push gate"), "{why}");
    // And the way out is named: the setting that asked for it.
    assert!(why.contains("Ask me before pushing"), "{why}");
    assert!(!repo.join(".pi_prompt").exists(), "the brief was never sent");
    let _ = std::fs::remove_dir_all(repo);
}

/// The gate asks about every `bash` call and the operator hears about pushes.
///
/// pi's hook cannot tell a push from a test run, so it asks about both; what
/// reaches the desk is decided here, by the reader every other door uses.
#[tokio::test]
async fn an_ordinary_pi_command_in_a_gated_job_asks_nobody() {
    stand_ins();
    let repo = a_repository("pi-gate-ordinary");
    std::fs::write(repo.join(".pi_command"), "npm test").unwrap();
    let path = repo.to_string_lossy().to_string();
    let (_controls, controlled) = tokio::sync::mpsc::channel(8);
    let (signals, mut heard) = tokio::sync::mpsc::channel(8);
    let env = guac_lib::secrets::Environment::default();
    let outcome = coding::run(
        job(Which::Pi, &path, "work", Gate::AskBeforePushing, &env),
        controlled,
        signals,
        |_| {},
    )
    .await
    .unwrap();
    assert!(outcome.failed.is_none());
    assert_eq!(std::fs::read_to_string(repo.join(".verdict")).unwrap(), "confirmed");
    assert!(
        !std::iter::from_fn(|| heard.try_recv().ok())
            .any(|signal| matches!(signal, coding::Signal::Permission { .. })),
        "nobody was asked"
    );
    let _ = std::fs::remove_dir_all(repo);
}

/// The three promises the bridge is built on, asked of the real program.
///
/// Not one of them is a flag, which is why this cannot be an offline test. Each
/// is a promise about how `claude` *behaves* when it is handed a hook, and the
/// offline suite can only check that Guaca said the right thing into a socket:
///
/// - a `PreToolUse` hook answering `deny` overrides `--permission-mode
///   bypassPermissions`, which is the mode every job here runs in, so the gate
///   is a gate rather than a suggestion;
/// - `additionalContext` from a `PostToolUse` hook, or a `Stop` hook's own
///   `reason`, is put in front of the model, so a correction typed into a
///   running job actually reaches it;
/// - an MCP server named on `--mcp-config` is reachable and its tools are
///   callable, so a job can report what it produced.
///
/// One model call covers all three: the brief asks for a push, which the gate
/// stops, and the staged message asks for a note, which only the bridge could
/// have delivered and only the MCP server could receive.
#[tokio::test]
#[ignore = "live: spends the operator's own Claude plan"]
async fn the_real_claude_still_honors_what_the_bridge_asks_of_it() {
    let repo = a_repository("live-bridge");
    std::fs::write(repo.join("a.txt"), "banana").unwrap();

    let bridge = coding::Bridge::new();
    let (signals, mut heard) = tokio::sync::mpsc::channel(32);
    let named = uuid::Uuid::new_v4().to_string();
    let session = bridge
        .open(signals, Gate::AskBeforePushing, repo.clone(), named.clone())
        .await
        .expect("the bridge has to start before anything else here means anything");

    // Staged before the job starts, so the first boundary it reaches has it.
    assert!(bridge.post(
        session.session_id(),
        "Before you do anything else, call the guaca note_progress tool with the note \
         `the mailbox works`.",
    ));

    let watching = tokio::spawn(async move {
        let (mut asked, mut noted) = (None, None);
        while let Some(signal) = heard.recv().await {
            match signal {
                // Answered `false`, which is the half that proves the override:
                // the run's own permission mode would have allowed this.
                coding::Signal::Permission { reach, reply, .. } => {
                    asked = Some(reach.what);
                    let _ = reply.send(false);
                }
                coding::Signal::Note(note) => noted = Some(note),
                coding::Signal::PullRequest { .. } | coding::Signal::Session(_) => {}
            }
        }
        (asked, noted)
    });

    let env = guac_lib::secrets::Environment::default();
    let (_controls, controlled) = tokio::sync::mpsc::channel(8);
    let (unused, _) = tokio::sync::mpsc::channel(8);
    let path = repo.to_string_lossy().to_string();
    let outcome = coding::run(
        coding::Job {
            session: &named,
            bridge: Some(&session),
            ..job(
                Which::Claude,
                &path,
                "Use the Bash tool to run: git push. Then use the Bash tool to run: echo hello. \
                 Then say in one sentence what happened.",
                Gate::AskBeforePushing,
                &env,
            )
        },
        controlled,
        unused,
        |_| {},
    )
    .await
    .expect("the harness has to start and answer");

    drop(session);
    let (asked, noted) = watching.await.unwrap();

    assert_eq!(outcome.failed, None, "the sign-in is spent or the vector is stale");
    // Chosen rather than read back off the stream, which is what makes it the
    // thing an operator can hand to `claude --resume` whatever the run did.
    assert_eq!(outcome.session_id, named);

    let asked = asked.expect(
        "a PreToolUse deny has to reach the desk. If this is None the program stopped \
         calling the hook, or stopped letting it refuse under bypassPermissions",
    );
    assert!(asked.contains("push"), "{asked}");

    let noted = noted.expect(
        "the staged message never reached the model, or the MCP server was not \
         reachable. Either way a job can no longer be corrected while it runs",
    );
    assert!(noted.to_lowercase().contains("mailbox"), "{noted}");

    let _ = std::fs::remove_dir_all(&repo);
}

/// A bridged job carries three more flags and keeps everything the operator has.
///
/// The offline half of the above, and it is the seam nothing else can see: drop
/// the wiring in `Runtime::start_job` and every other suite in this repo still
/// passes while no job in any workspace can be reached again.
#[tokio::test]
async fn a_bridged_job_is_started_with_its_own_session_hooks_and_server() {
    let repo = a_repository("bridged-argv");
    stand_ins();

    let bridge = coding::Bridge::new();
    let (signals, _heard) = tokio::sync::mpsc::channel(8);
    let named = uuid::Uuid::new_v4().to_string();
    let session = bridge
        .open(signals.clone(), Gate::AskBeforePushing, repo.clone(), named.clone())
        .await
        .unwrap();

    let env = guac_lib::secrets::Environment::default();
    let (_controls, controlled) = tokio::sync::mpsc::channel(8);
    let path = repo.to_string_lossy().to_string();
    coding::run(
        coding::Job {
            session: &named,
            bridge: Some(&session),
            ..job(Which::Claude, &path, "do the thing", Gate::AskBeforePushing, &env)
        },
        controlled,
        signals,
        |_| {},
    )
    .await
    .unwrap();

    let argv = argv_at(&repo);
    let after = |flag: &str| {
        argv.iter().position(|arg| arg == flag).and_then(|at| argv.get(at + 1)).cloned()
    };

    // Chosen rather than read back, which is what makes `claude --resume` open
    // *this* job rather than whatever ran last in the directory.
    assert_eq!(after("--session-id").as_deref(), Some(session.session_id()));

    // The hooks are a real file on disk with this job's own address in them,
    // and the script has to be executable or the program cannot run it.
    let settings = after("--settings").expect("a bridged job carries its hooks");
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    for event in ["PostToolUse", "Stop", "PreToolUse"] {
        assert!(written["hooks"][event].is_array(), "{event}");
    }
    let script = written["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(script).unwrap().permissions());
    assert_eq!(mode & 0o111, 0o100, "the hook has to be runnable, and by nobody else");
    assert!(std::fs::read_to_string(script).unwrap().contains(session.session_id()));

    // Added to the operator's own setup rather than replacing it. A coding job
    // in their repository wants their rules file and their servers, which is
    // the opposite of what a turn wants and right for the opposite reason.
    assert!(after("--mcp-config").unwrap().contains("guaca"));
    assert!(!argv.contains(&"--strict-mcp-config".to_string()));
    assert!(!argv.contains(&"--setting-sources".to_string()));

    // And the scratch goes when the job does, so a token that reached a running
    // job cannot be read off the disk afterward.
    let dir = std::path::PathBuf::from(&settings).parent().unwrap().to_path_buf();
    drop(session);
    assert!(!dir.exists());
    let _ = std::fs::remove_dir_all(&repo);
}

/// The same question of `pi`.
#[tokio::test]
#[ignore = "live: spends whatever pi is signed in to"]
async fn the_real_pi_still_answers_the_way_this_build_reads() {
    let repo = a_repository("live-pi");
    std::fs::write(repo.join("a.txt"), "banana").unwrap();

    let outcome = run_job(
        Which::Pi,
        &repo,
        "Read a.txt and say what one word it contains. Change nothing and commit nothing.",
        |_| {},
    )
    .await
    .expect("the harness has to start and answer");

    assert_eq!(outcome.failed, None, "the sign-in is spent or the vector is stale");
    assert!(outcome.said.to_lowercase().contains("banana"), "{}", outcome.said);
    assert!(outcome.tool_calls > 0, "it has to have read the file rather than guessed");
    let _ = std::fs::remove_dir_all(&repo);
}

/// The real Claude Code stops on the SDK's interrupt, and the session it
/// stopped in carries on.
///
/// The interrupt is a promise about how the program behaves when it is sent a
/// line on stdin, which no stand-in can check. Measured against 2.1.260 and
/// 2.1.283: a `control_response`, then `result` with `terminal_reason`
/// `aborted_streaming`, and `--resume` on the same id remembers the brief.
#[tokio::test]
#[ignore = "live: spends the operator's own Claude plan, on haiku"]
async fn the_real_claude_stops_on_an_interrupt_and_the_session_carries_on() {
    let repo = a_repository("live-interrupt");
    let session = uuid::Uuid::new_v4().to_string();
    let path = repo.to_string_lossy().to_string();
    let tuning = Tuning { model: Some("haiku".into()), ..Tuning::default() };
    let env = guac_lib::secrets::Environment::default();

    let (controls, controlled) = tokio::sync::mpsc::channel(8);
    let (signals, _heard) = tokio::sync::mpsc::channel(8);
    let (used, mut using) = tokio::sync::mpsc::unbounded_channel();
    let running = coding::run(
        coding::Job {
            session: &session,
            tuning: &tuning,
            ..job(Which::Claude, &path, "Use the Bash tool to run exactly `sleep 60 && echo finished` in the foreground, not in the background, and wait for it. Then reply with the single word done.", Gate::Open, &env)
        },
        controlled,
        signals,
        move |progress| {
            if matches!(progress, Progress::Using { .. }) {
                let _ = used.send(());
            }
        },
    );
    let stopping = async {
        using.recv().await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        controls.send(coding::Control::Stop).await.unwrap();
    };
    let (outcome, ()) = tokio::time::timeout(std::time::Duration::from_secs(40), async {
        tokio::join!(running, stopping)
    })
    .await
    .expect("an interrupted job ends within the grace, well before the sleep");
    let outcome = outcome.unwrap();
    assert!(outcome.stopped, "{outcome:?}");
    assert_eq!(outcome.failed, None, "a stop that was asked for is not a failure");

    let (_controls, controlled) = tokio::sync::mpsc::channel(8);
    let (signals, _heard) = tokio::sync::mpsc::channel(8);
    let resumed = coding::run(
        coding::Job {
            session: &session,
            resume: true,
            tuning: &tuning,
            ..job(Which::Claude, &path, "In five words or fewer, and without using any tool: what command were you asked to run?", Gate::Open, &env)
        },
        controlled,
        signals,
        |_| {},
    )
    .await
    .unwrap();
    assert!(resumed.said.to_lowercase().contains("sleep"), "{}", resumed.said);
    let _ = std::fs::remove_dir_all(&repo);
}

/// The real `pi` loads the provider Guaca writes, and pays through the relay.
///
/// Against a stand-in endpoint on loopback, so it spends nothing: what is being
/// asked is whether pi honors `registerProvider` from `-e` as this build writes
/// it, and sends the token rather than anything else.
#[tokio::test]
#[ignore = "live: needs pi on PATH; spends nothing"]
async fn the_real_pi_pays_through_the_relay_and_never_holds_the_key() {
    use axum::{routing::post, Router};
    let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let recorder = seen.clone();
    let upstream = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: axum::http::HeaderMap, body: axum::extract::Json<serde_json::Value>| {
            let recorder = recorder.clone();
            async move {
                let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
                recorder.lock().push(format!("{auth} {}", body.0["model"]));
                let chunk = |delta: serde_json::Value, finish: serde_json::Value| {
                    format!("data: {}\n\n", serde_json::json!({"id":"x","object":"chat.completion.chunk","model":body.0["model"],"choices":[{"index":0,"delta":delta,"finish_reason":finish}]}))
                };
                let stream = format!(
                    "{}{}data: [DONE]\n\n",
                    chunk(serde_json::json!({"role":"assistant","content":"relayed hello"}), serde_json::Value::Null),
                    chunk(serde_json::json!({}), serde_json::json!("stop")),
                );
                ([("content-type", "text/event-stream")], stream)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });

    let relay = coding::relay::Relay::new();
    let lease = relay
        .lend(
            coding::relay::Upstream {
                base_url: format!("http://{address}/v1"),
                api_key: "sk-the-real-one".into(),
                referer: "https://example.com".into(),
                title: "Guac".into(),
            },
            None,
        )
        .await
        .unwrap();
    let repo = a_repository("live-relay");
    let path = repo.to_string_lossy().to_string();
    let tuning =
        Tuning { model: Some("stand-in/model".into()), pays: Payer::GuacaKey, ..Tuning::default() };
    let env = guac_lib::secrets::Environment::default();
    let (_controls, controlled) = tokio::sync::mpsc::channel(8);
    let (signals, _heard) = tokio::sync::mpsc::channel(8);
    let outcome = coding::run(
        coding::Job {
            tuning: &tuning,
            lent: Some(&lease),
            ..job(Which::Pi, &path, "Say hello.", Gate::Open, &env)
        },
        controlled,
        signals,
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(outcome.failed, None, "{outcome:?}");
    assert_eq!(outcome.said, "relayed hello");
    assert_eq!(*seen.lock(), [r#"Bearer sk-the-real-one "stand-in/model""#]);
    let _ = std::fs::remove_dir_all(&repo);
}

/// The real programs list their models, and a listing makes no model call.
#[tokio::test]
#[ignore = "live: needs claude, codex and pi on PATH; spends nothing"]
async fn the_real_harnesses_list_their_models() {
    for which in Which::ALL {
        let offers =
            coding::models(which, None).await.unwrap_or_else(|err| panic!("{which:?}: {err}"));
        assert!(!offers.is_empty(), "{which:?} listed nothing");
        assert!(offers.iter().all(|offer| !offer.id.is_empty() && !offer.id.starts_with('-')));
        if which != Which::Pi {
            assert!(
                offers.iter().any(|offer| offer.default),
                "{which:?} marks what runs by default"
            );
        }
    }
}

// ---- where a job works ---------------------------------------------------

/// A job works in the repository it names, inside the agent's terminal.
///
/// The ordinary arrangement: an agent clones what it works on into a directory
/// of its own and hands `code` that directory. The stand-in records its
/// argument vector into wherever it was started, so a recording in the
/// terminal's top level rather than in the repository is a job started in the
/// wrong place.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_runs_in_the_directory_it_names_inside_the_agents_terminal() {
    stand_ins();
    let stub = serve(|body| {
        if anyone_said(body, "has finished") {
            Script::Say("It is done.".into())
        } else {
            Script::CodeIn { task: "fix the flaky test".into(), directory: "guaca".into() }
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let card = h.agent_named("Engineer").unwrap();
    h.runtime.store().set_has_terminal(card.id, true).unwrap();
    h.runtime.store().set_agent_coding(card.id, Which::Claude, Gate::Open).unwrap();
    let home = h.runtime.terminals().ensure(card.id).unwrap();
    let repo = home.join("guaca");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);

    let run = h.runtime.send_from_human(card.id, "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the job comes back", |h| {
        h.channel_texts("Engineer").iter().any(|line| line.contains("It is done"))
    })
    .await;

    assert!(repo.join(ARGV).exists(), "the harness did not run in the repository it named");
    assert!(!home.join(ARGV).exists(), "and it did not run at the top of the terminal");
    let started = h
        .sink
        .count_of(|event| matches!(event, UiEvent::CodingJobStarted { directory, .. } if directory == "guaca"));
    assert_eq!(started, 1, "the panel is told where the job is working");
}

/// A directory that is not in the terminal is refused in the turn that asked,
/// before anything is started, and one that is not a repository is said to be
/// one with no undo.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_is_refused_outside_the_terminal_and_warned_outside_a_repository() {
    stand_ins();
    let stub = serve(|body| {
        let acted = body["messages"]
            .as_array()
            .map(|messages| messages.iter().filter(|m| m["role"] == "tool").count())
            .unwrap_or(0);
        match acted {
            0 => Script::CodeIn { task: "look around".into(), directory: "..".into() },
            1 => Script::CodeIn { task: "look around".into(), directory: "missing".into() },
            2 => Script::Code("scaffold a site".into()),
            _ => Script::Say("Started it.".into()),
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let card = h.agent_named("Engineer").unwrap();
    h.runtime.store().set_has_terminal(card.id, true).unwrap();
    let home = h.runtime.terminals().ensure(card.id).unwrap();
    // Held open, so the report of a finished job does not start a turn whose
    // request is the last one the stub saw.
    std::fs::write(home.join(LINGER), "30").unwrap();

    let run = h.runtime.send_from_human(card.id, "start something").unwrap();
    h.settle(run).await;
    h.wait_until("the brief is read", |_| home.join(".pi_prompt").exists()).await;

    let told = last_tool_results(&stub);
    assert!(told[0].contains("outside your terminal"), "{told:?}");
    assert!(told[1].contains("not a directory in your terminal"), "{told:?}");
    assert!(told[2].contains("A coding agent is working"), "{told:?}");
    let brief = brief_seen(Which::Pi, &home);
    assert!(brief.contains("not a git repository"), "{brief}");
    assert!(brief.contains("git init"), "{brief}");
    h.runtime.stop_job(card.id, Origin::Operator).unwrap();
}

/// Two agents in one crew each run a job at once, each in its own terminal.
///
/// One job per agent is the whole lock. Two changes at once are two agents,
/// and neither holds the other out.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_agents_in_one_crew_both_get_a_job() {
    stand_ins();
    let stub = serve(|body| match anyone_said(body, "A coding agent is working") {
        true => Script::Say("It is under way.".into()),
        false => Script::Code("fix the flaky test".into()),
    })
    .await;
    let h = harness(&stub, &["Engineer", "Reviewer"], GuardLimits::default());
    let mut homes = Vec::new();
    for (name, label) in [("Engineer", "both-a"), ("Reviewer", "both-b")] {
        let repo = a_repository(label);
        // Long enough that both jobs are genuinely in flight together.
        std::fs::write(repo.join(LINGER), "30").unwrap();
        homes.push(give_terminal(&h, name, repo, Which::Claude, Gate::Open));
    }

    for name in ["Engineer", "Reviewer"] {
        let run = h.runtime.send_from_human(h.id(name), "fix the flaky test").unwrap();
        h.settle(run).await;
    }
    h.wait_until("both harnesses are up", |_| homes.iter().all(|home| home.join(ARGV).exists()))
        .await;
    assert!(!h.transcript().contains("already have a coding agent"), "{}", h.transcript());

    for name in ["Engineer", "Reviewer"] {
        let _ = h.runtime.stop_job(h.id(name), Origin::Operator);
    }
}

// ---- secrets -------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_commands_receive_only_granted_secrets_and_refresh_after_rotation() {
    use guac_lib::domain::connector::CleanConnector;
    let repo = a_repository("secret-runtime");
    let stub = serve(|body| {
        if body["messages"].as_array().unwrap().iter().rev().take_while(|m| m["role"] != "user").any(|m| m["role"] == "tool") {
            Script::Say("Checked.".into())
        } else {
            Script::Shell(
                "printf '%s\\n' \"${CLOUDFLARE_API_TOKEN:-missing}\"; printf '%s' \"${CLOUDFLARE_API_TOKEN:-missing}\" | shasum -a 256".into(),
            )
        }
    })
    .await;
    let h = harness(&stub, &["Deployer", "Researcher"], GuardLimits::default());
    let repo = give_terminal(&h, "Deployer", repo, Which::Claude, Gate::Open);
    let _theirs = give_terminal(
        &h,
        "Researcher",
        a_repository("secret-runtime-researcher"),
        Which::Claude,
        Gate::Open,
    );
    let agent = h.runtime.store().get_agent(h.id("Deployer")).unwrap().unwrap();
    let saved = h
        .runtime
        .store()
        .create_connector(&CleanConnector {
            group_id: agent.group_id,
            service: "Cloudflare".into(),
            account: String::new(),
            env_var: "CLOUDFLARE_API_TOKEN".into(),
            note: String::new(),
            secret: "private-cloudflare-fixture".into(),
            agents: vec![agent.id],
        })
        .unwrap();
    let run = h.runtime.send_from_human(agent.id, "Check access").unwrap();
    h.settle(run).await;
    let results = tool_results(&stub).join("\n");
    assert!(results.contains("[REDACTED]"), "{results}");
    assert!(!results.contains("private-cloudflare-fixture"));
    let run = h.runtime.send_from_human(h.id("Researcher"), "Check access").unwrap();
    h.settle(run).await;
    assert!(tool_results(&stub).join("\n").contains("missing"));
    h.runtime.store().update_connector(saved.id, &[agent.id], Some("new-private-value")).unwrap();
    let run = h.runtime.send_from_human(agent.id, "Check rotated access").unwrap();
    h.settle(run).await;
    use sha2::{Digest, Sha256};
    let expected = format!("{:x}", Sha256::digest(b"new-private-value"));
    assert!(tool_results(&stub).last().unwrap().contains(&expected));
    h.runtime.store().update_connector(saved.id, &[], None).unwrap();
    let run = h.runtime.send_from_human(agent.id, "Check revoked access").unwrap();
    h.settle(run).await;
    assert!(tool_results(&stub).last().unwrap().contains("missing"));
    assert!(h.runtime.store().connector_env(agent.id).unwrap().is_empty());
    assert!(!h.transcript().contains("private-cloudflare-fixture"));
    let _ = std::fs::remove_dir_all(repo);
}

#[tokio::test]
async fn every_coding_harness_receives_secrets_without_putting_values_in_guaca_output() {
    stand_ins();
    for (which, name) in
        [(Which::Pi, "secret-pi"), (Which::Claude, "secret-claude"), (Which::Codex, "secret-codex")]
    {
        let repo = a_repository(name);
        std::fs::write(repo.join(".secret_probe"), "").unwrap();
        let env = guac_lib::secrets::Environment {
            names: vec!["CLOUDFLARE_API_TOKEN".into()],
            values: std::collections::BTreeMap::from([(
                "CLOUDFLARE_API_TOKEN".into(),
                "private-harness-fixture".into(),
            )]),
        };
        let mut progress = Vec::new();
        let done = run_job_env(which, &repo, "check", &env, |p| progress.push(p)).await.unwrap();
        assert!(done.failed.is_none(), "{which:?}");
        assert!(done.said.contains("[REDACTED]"), "{which:?}: {}", done.said);
        assert!(!format!("{progress:?} {done:?}").contains("private-harness-fixture"));
        let recorded = brief_seen(which, &repo);
        assert!(recorded.contains("CLOUDFLARE_API_TOKEN"));
        assert!(!recorded.contains("private-harness-fixture"));
        let _ = std::fs::remove_dir_all(repo);
    }
}

#[tokio::test]
#[ignore = "live: checks all three signed-in harnesses with synthetic credentials"]
async fn real_harness_shells_receive_the_granted_environment() {
    use sha2::{Digest, Sha256};
    let mut failures = Vec::new();
    for which in Which::ALL {
        let repo = a_repository(&format!("live-secret-{}", which.as_str()));
        std::fs::write(repo.join("check-secret.py"),
            "import hashlib, os\nfrom pathlib import Path\nPath('secret-result.txt').write_text(hashlib.sha256(os.environ.get('CLOUDFLARE_API_TOKEN', '').encode()).hexdigest())\n").unwrap();
        let value = format!("synthetic-{}", uuid::Uuid::new_v4());
        let expected = format!("{:x}", Sha256::digest(value.as_bytes()));
        let env = guac_lib::secrets::Environment {
            names: vec!["CLOUDFLARE_API_TOKEN".into()],
            values: std::collections::BTreeMap::from([("CLOUDFLARE_API_TOKEN".into(), value)]),
        };
        let result = tokio::time::timeout(std::time::Duration::from_secs(180), run_job_env(
            which, &repo,
            "Run python3 check-secret.py once using your shell tool. Do not edit the script or inspect environment values. Do not commit, use other tools, or delegate. Then say done.",
            &env, |_| {},
        )).await;
        let passed = matches!(result, Ok(Ok(_)))
            && std::fs::read_to_string(repo.join("secret-result.txt")).ok().as_deref()
                == Some(&expected);
        if !passed {
            failures.push(format!("{}: {result:?}", which.as_str()));
        }
        let _ = std::fs::remove_dir_all(repo);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_runtime_coding_job_uses_the_agents_secret_grant() {
    stand_ins();
    let repo = a_repository("secret-runtime-job");
    std::fs::write(repo.join(".secret_probe"), "").unwrap();
    let stub = serve(|body| {
        if anyone_said(body, "has finished") {
            Script::Say("Secret job complete.".into())
        } else if anyone_said(body, "could not finish") {
            Script::Say("Secret job failed.".into())
        } else {
            Script::Code("Check deployment access".into())
        }
    })
    .await;
    let h = harness(&stub, &["Deployer"], GuardLimits::default());
    let repo = give_terminal(&h, "Deployer", repo, Which::Claude, Gate::Open);
    let agent = h.agent_named("Deployer").unwrap();
    h.runtime
        .store()
        .create_connector(&guac_lib::domain::connector::CleanConnector {
            group_id: agent.group_id,
            service: "Cloudflare".into(),
            account: String::new(),
            env_var: "CLOUDFLARE_API_TOKEN".into(),
            note: String::new(),
            secret: "private-runtime-fixture".into(),
            agents: vec![agent.id],
        })
        .unwrap();
    let run = h.runtime.send_from_human(agent.id, "Check deployment access").unwrap();
    h.settle(run).await;
    h.wait_until("the job answers", |h| {
        h.channel_texts("Deployer").iter().any(|line| line.contains("Secret job"))
    })
    .await;
    let text = h.transcript();
    assert!(text.contains("Secret job complete."), "{text}");
    assert!(text.contains("[REDACTED]"), "{text}");
    assert!(!text.contains("private-runtime-fixture"));
    let _ = std::fs::remove_dir_all(repo);
}
