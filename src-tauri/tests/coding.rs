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

use std::io::Write;
use std::path::{Path, PathBuf};

use guac_lib::coding::{self, Progress};
use guac_lib::domain::approval::Decision;
use guac_lib::domain::repository::{CleanRepository, Gate, Harness as Which};
use guac_lib::runtime::guard::GuardLimits;

use harness::*;

/// Where a stand-in records what it was called with. Inside the repository it
/// was run in, which is what makes it per-test: two tests run concurrently in
/// one binary and share one `PATH`.
const ARGV: &str = ".argv";

/// What a stand-in prints, if the test wrote one. Otherwise it answers with the
/// canned success below.
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

/// A directory holding both stand-ins, put on `PATH` exactly once.
///
/// Once, because `PATH` is process-wide and these tests run concurrently:
/// writing it per test is a read racing a write in another thread. Written
/// before any test body runs anything that looks it up, and never again.
fn stand_ins() -> &'static Path {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        write_stand_in(dir.path(), "pi", PI_SUCCESS);
        write_stand_in(dir.path(), "claude", CLAUDE_SUCCESS);
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{path}", dir.path().display()));
        dir
    });
    dir.path()
}

/// A stand-in: records its arguments, then prints a stream.
///
/// One argument per line in the recording, because a brief and a system prompt
/// both contain spaces and newlines and a flat join could not be read back.
fn write_stand_in(dir: &Path, name: &str, canned: &str) {
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$1\" = '--version' ]; then echo 'stand-in'; exit 0; fi\n\
         : > {ARGV}\n\
         for arg in \"$@\"; do printf '%s\\n<<>>\\n' \"$arg\" >> {ARGV}; done\n\
         if [ -f {LINGER} ]; then sleep \"$(cat {LINGER})\"; fi\n\
         if [ -f {SAY} ]; then cat {SAY}; fi\n\
         if [ -f {EXIT} ]; then exit \"$(cat {EXIT})\"; fi\n\
         if [ -f {SAY} ]; then exit 0; fi\n\
         cat <<'STREAM'\n{canned}\nSTREAM\n"
    );
    let at = dir.join(name);
    let mut file = std::fs::File::create(&at).unwrap();
    file.write_all(script.as_bytes()).unwrap();
    drop(file);
    std::fs::set_permissions(&at, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

const PI_SUCCESS: &str = concat!(
    r#"{"type":"tool_execution_start","toolName":"bash","args":{"command":"npm test"}}"#,
    "\n",
    r#"{"type":"message_end","message":{"role":"assistant","model":"gpt-5.6","content":[{"type":"text","text":"Fixed the flaky test and pushed."}],"stopReason":"stop"}}"#,
);

const CLAUDE_SUCCESS: &str = concat!(
    r#"{"type":"system","subtype":"init","model":"claude-opus-5"}"#,
    "\n",
    r#"{"type":"assistant","message":{"model":"claude-opus-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]}}"#,
    "\n",
    r#"{"type":"result","subtype":"success","is_error":false,"result":"Fixed the flaky test and pushed.","total_cost_usd":0.12}"#,
);

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

    let outcome =
        coding::run(Which::Claude, repo.to_str().unwrap(), "fix the flaky test", None, |_| {})
            .await
            .unwrap();

    let argv = argv_at(&repo);
    // The brief reaches the program as an argument, not as something it has to
    // go and find: the harness cannot see the conversation it came from.
    assert!(argv.contains(&"fix the flaky test".to_string()), "{argv:?}");
    // Claude Code's own vector, and this is the half no unit test can check:
    // the CLI refuses `stream-json` without `--verbose`, which is a job that
    // never starts rather than a job that fails.
    assert!(argv.contains(&"stream-json".to_string()), "{argv:?}");
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
    let outcome = coding::run(Which::Pi, repo.to_str().unwrap(), "fix the flaky test", None, |p| {
        seen.push(p)
    })
    .await
    .unwrap();

    let argv = argv_at(&repo);
    assert!(argv.contains(&"--mode".to_string()), "{argv:?}");
    assert!(argv.contains(&"json".to_string()), "{argv:?}");
    assert!(!argv.contains(&"stream-json".to_string()), "{argv:?}");
    assert_eq!(outcome.said, "Fixed the flaky test and pushed.");
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
        coding::run(which, repo.to_str().unwrap(), "do the thing", None, |_| {}).await.unwrap();
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

        let outcome = coding::run(which, repo.to_str().unwrap(), "do the thing", None, |_| {})
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

    let err = coding::run(Which::Pi, repo.to_str().unwrap(), "do the thing", None, |_| {})
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
/// It is covered where it is cheap and where it matters: `RepositoryList`'s
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
    // `pi` has no second interface at all, so it is never bridged whatever it
    // says its version is.
    assert!(matches!(
        coding::presence(Which::Pi).await,
        coding::Presence::Installed { bridged: false, .. }
    ));
}

// ---- the whole path ------------------------------------------------------

/// The agent that asked is told what the harness said, in its own channel.
///
/// This is the test that reads the column. A `code` call in a repository set to
/// Claude Code has to start `claude`, and the answer has to come back to the
/// agent as a message on a fresh run, minutes after the turn that asked for it
/// ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_is_told_what_the_harness_it_was_given_said() {
    stand_ins();
    let repo = a_repository("end-to-end");

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

    let engineer = h.agent_named("Engineer").unwrap();
    let linked = h
        .runtime
        .store()
        .create_repository(&CleanRepository {
            group_id: engineer.group_id,
            name: "guaca".into(),
            path: repo.to_string_lossy().to_string(),
            note: String::new(),
            harness: Which::Claude,
            gate: Gate::Open,
        })
        .unwrap();
    h.runtime.store().set_agent_repository(engineer.id, Some(linked.id)).unwrap();

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;

    // The job outlives the turn that started it, which is the whole shape of
    // this feature: the tool returns as soon as the process is up.
    h.wait_until("the coding job is reported back", |h| {
        h.channel_texts("Engineer").iter().any(|line| line.contains("fixed the flaky test"))
    })
    .await;

    let argv = argv_at(&repo);
    assert!(argv.contains(&"stream-json".to_string()), "the column was not read: {argv:?}");
    // Contained rather than equal: the brief a job is started with carries the
    // footing in front of it, which the test below is the test of.
    assert!(argv.iter().any(|arg| arg.contains("fix the flaky test")), "{argv:?}");
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

    let stub = serve(|body| {
        if anyone_said(body, "has finished") {
            Script::Say("The coding agent did the work.".into())
        } else {
            Script::Code("fix the flaky test".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());

    let engineer = h.agent_named("Engineer").unwrap();
    let linked = h
        .runtime
        .store()
        .create_repository(&CleanRepository {
            group_id: engineer.group_id,
            name: "guaca".into(),
            path: repo.to_string_lossy().to_string(),
            note: "run ./scripts/ci.sh before you finish".into(),
            harness: Which::Pi,
            gate: Gate::Open,
        })
        .unwrap();
    h.runtime.store().set_agent_repository(engineer.id, Some(linked.id)).unwrap();

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the coding job is reported back", |h| {
        h.channel_texts("Engineer").iter().any(|line| line.contains("did the work"))
    })
    .await;

    let argv = argv_at(&repo);
    let brief = argv
        .iter()
        .find(|arg| arg.contains("fix the flaky test"))
        .unwrap_or_else(|| panic!("the brief never reached the program: {argv:?}"));

    // The state, the rule it resolves to, the work, and the operator's note, in
    // that order. The footing leads because it is read before the first edit or
    // it is not read at all.
    assert!(brief.contains("On branch `landed`"), "{brief}");
    assert!(brief.contains("already contained in `main`"), "{brief}");
    assert!(brief.contains("start from `main`"), "{brief}");
    let state = brief.find("Where you are starting from").expect("no footing: {brief}");
    let work = brief.find("fix the flaky test").unwrap();
    let note = brief.find("Standing instruction").expect("the note still rides along");
    assert!(state < work && work < note, "the three parts are out of order: {brief}");

    let _ = std::fs::remove_dir_all(&repo);
}

// ---- the other door ------------------------------------------------------

/// Links a repository to an agent and answers with where it is on disk.
fn put_in_a_repository(h: &Harness, agent: &str, repo: &Path, gate: Gate) {
    let card = h.agent_named(agent).unwrap();
    let linked = h
        .runtime
        .store()
        .create_repository(&CleanRepository {
            group_id: card.group_id,
            name: "guaca".into(),
            path: repo.to_string_lossy().to_string(),
            note: String::new(),
            harness: Which::Claude,
            gate,
        })
        .unwrap();
    h.runtime.store().set_agent_repository(card.id, Some(linked.id)).unwrap();
}

/// The small door, end to end: a real shell, in the operator's own repository,
/// answering inside the turn that asked.
///
/// This is the seam nothing else can see. `shell` is offered on the same
/// condition as `code` and reads the same column, and the failure it exists to
/// stop is not a crash: it is an agent that has to spend a coding job, minutes
/// and somebody's plan on `git status`, and that reports having no shell at all
/// when the harness will not start.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_in_a_repository_runs_a_line_there_and_is_answered_in_the_same_turn() {
    let repo = a_repository("shell-here");
    let here = repo.to_string_lossy().to_string();

    // Branched on the tool result rather than on a counter, because a turn can
    // take more than one call. The needle is the repository's own path, which
    // is the whole assertion: a shell that ran somewhere else answers with
    // somewhere else.
    let stub = serve(move |body| {
        if anyone_said(body, &here) {
            Script::Say("I am standing in the repository.".into())
        } else {
            Script::InRepository("git rev-parse --show-toplevel".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    put_in_a_repository(&h, "Engineer", &repo, Gate::Open);

    let run = h.runtime.send_from_human(h.id("Engineer"), "which directory are you in?").unwrap();
    h.settle(run).await;

    let told = tool_results(&stub).join("\n");
    assert!(
        told.contains(&repo.to_string_lossy().to_string()),
        "the line did not run in the repository:\n{told}"
    );
    assert!(
        h.channel_texts("Engineer").iter().any(|t| t.contains("standing in the repository")),
        "and the turn finished on it:\n{}",
        h.transcript()
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// The gate is a fact about the repository, so it cannot mean one thing through
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
            Script::InRepository("touch pushed.txt && git push origin main".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    put_in_a_repository(&h, "Engineer", &repo, Gate::AskBeforePushing);

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
            Script::InRepository("git status --porcelain; echo read-the-tree".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    put_in_a_repository(&h, "Engineer", &repo, Gate::AskBeforePushing);

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

/// The door that stays open when the other one will not.
///
/// A work tree with a job already in it refuses `code`, on purpose: two
/// harnesses in one directory interleave their edits. One line is not that, and
/// refusing it here would take away the read an agent most wants while a job
/// runs, which is what the job is doing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_line_still_runs_in_a_work_tree_a_coding_job_is_already_in() {
    stand_ins();
    let repo = a_repository("shell-alongside");
    // Slow enough that the job is genuinely still running when the line does.
    std::fs::write(repo.join(LINGER), "3").unwrap();

    let stub = serve(|body| {
        if anyone_said(body, "alongside-the-job") {
            Script::Say("The job is still going.".into())
        } else if has_tool_result(body) {
            Script::InRepository("echo alongside-the-job".into())
        } else {
            Script::Code("something long".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    put_in_a_repository(&h, "Engineer", &repo, Gate::Open);

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

    let outcome = coding::run(
        Which::Claude,
        repo.to_str().unwrap(),
        "Read a.txt and say what one word it contains. Change nothing and commit nothing.",
        None,
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
    let repo = a_repository("stopped");
    // Long enough that the test reaches it while it is still running, and
    // short enough that a broken stop fails the test rather than hanging it.
    std::fs::write(repo.join(LINGER), "30").unwrap();

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
    let linked = h
        .runtime
        .store()
        .create_repository(&CleanRepository {
            group_id: engineer.group_id,
            name: "guaca".into(),
            path: repo.to_string_lossy().to_string(),
            note: String::new(),
            harness: Which::Claude,
            gate: Gate::Open,
        })
        .unwrap();
    h.runtime.store().set_agent_repository(engineer.id, Some(linked.id)).unwrap();

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the harness is up", |_| repo.join(ARGV).exists()).await;

    h.runtime.stop_job(linked.id).expect("a running job has to be stoppable");

    // Told, rather than left waiting for a message that is not coming. An agent
    // that is never told answers "I started that and have not heard back",
    // which is true and useless.
    h.wait_until("the agent is told it was stopped", |h| {
        h.channel_texts("Engineer").iter().any(|line| line.contains("stopped the coding agent"))
    })
    .await;

    // And the lane is free, so the next brief does not come back busy about a
    // job that is over.
    h.runtime.message_job(linked.id, "anything").expect_err("a stopped job is not a running one");

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
    let repo = a_repository("stop-twice");
    let linked = h
        .runtime
        .store()
        .create_repository(&CleanRepository {
            group_id: engineer.group_id,
            name: "guaca".into(),
            path: repo.to_string_lossy().to_string(),
            note: String::new(),
            harness: Which::Pi,
            gate: Gate::Open,
        })
        .unwrap();

    let why = h.runtime.stop_job(linked.id).unwrap_err().to_string();
    assert!(why.contains("already finished"), "{why}");
    let _ = std::fs::remove_dir_all(&repo);
}

/// A `pi` job says why it cannot be reached, rather than accepting a message
/// nothing will read.
///
/// The two ways of being unreachable have opposite answers for the operator,
/// which is why they are different sentences: one is worth waiting a moment
/// for and the other is a fact about the repository.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_harness_with_no_second_interface_says_so_instead_of_swallowing_it() {
    stand_ins();
    let repo = a_repository("unreachable");
    std::fs::write(repo.join(LINGER), "30").unwrap();

    let stub = serve(|body| {
        if anyone_said(body, "has finished") {
            Script::Say("done".into())
        } else {
            Script::Code("fix the flaky test".into())
        }
    })
    .await;
    let h = harness(&stub, &["Engineer"], GuardLimits::default());
    let engineer = h.agent_named("Engineer").unwrap();
    let linked = h
        .runtime
        .store()
        .create_repository(&CleanRepository {
            group_id: engineer.group_id,
            name: "guaca".into(),
            path: repo.to_string_lossy().to_string(),
            note: String::new(),
            harness: Which::Pi,
            gate: Gate::Open,
        })
        .unwrap();
    h.runtime.store().set_agent_repository(engineer.id, Some(linked.id)).unwrap();

    let run = h.runtime.send_from_human(h.id("Engineer"), "fix the flaky test").unwrap();
    h.settle(run).await;
    h.wait_until("the harness is up", |_| repo.join(ARGV).exists()).await;

    let why = h.runtime.message_job(linked.id, "use the other endpoint").unwrap_err().to_string();
    assert!(why.contains("pi"), "{why}");
    // The way out is named, because an operator cannot guess it from a message
    // about a harness.
    assert!(why.contains("Claude Code"), "{why}");

    h.runtime.stop_job(linked.id).unwrap();
    let _ = std::fs::remove_dir_all(&repo);
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
    let session = bridge
        .open(signals, Gate::AskBeforePushing)
        .await
        .expect("the bridge has to start before anything else here means anything");
    let named = session.session_id().to_string();

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
                coding::Signal::Permission { command, reply } => {
                    asked = Some(command);
                    let _ = reply.send(false);
                }
                coding::Signal::Note(note) => noted = Some(note),
                coding::Signal::PullRequest { .. } => {}
            }
        }
        (asked, noted)
    });

    let outcome = coding::run(
        Which::Claude,
        repo.to_str().unwrap(),
        "Use the Bash tool to run: git push. Then use the Bash tool to run: echo hello. \
         Then say in one sentence what happened.",
        Some(session.wiring()),
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
    let session = bridge.open(signals, Gate::AskBeforePushing).await.unwrap();

    coding::run(
        Which::Claude,
        repo.to_str().unwrap(),
        "do the thing",
        Some(session.wiring()),
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

    let outcome = coding::run(
        Which::Pi,
        repo.to_str().unwrap(),
        "Read a.txt and say what one word it contains. Change nothing and commit nothing.",
        None,
        |_| {},
    )
    .await
    .expect("the harness has to start and answer");

    assert_eq!(outcome.failed, None, "the sign-in is spent or the vector is stale");
    assert!(outcome.said.to_lowercase().contains("banana"), "{}", outcome.said);
    assert!(outcome.tool_calls > 0, "it has to have read the file rather than guessed");
    let _ = std::fs::remove_dir_all(&repo);
}
