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
use guac_lib::domain::repository::{CleanRepository, Harness as Which};
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

    let outcome = coding::run(Which::Claude, repo.to_str().unwrap(), "fix the flaky test", |_| {})
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
    let outcome =
        coding::run(Which::Pi, repo.to_str().unwrap(), "fix the flaky test", |p| seen.push(p))
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
        coding::run(which, repo.to_str().unwrap(), "do the thing", |_| {}).await.unwrap();
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

        let outcome = coding::run(which, repo.to_str().unwrap(), "do the thing", |_| {})
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

    let err = coding::run(Which::Pi, repo.to_str().unwrap(), "do the thing", |_| {})
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
    assert!(coding::installed(Which::Pi).await);
    assert!(coding::installed(Which::Claude).await);
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
    assert!(argv.contains(&"fix the flaky test".to_string()), "{argv:?}");
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
        |_| {},
    )
    .await
    .expect("the harness has to start and answer");

    assert_eq!(outcome.failed, None, "the sign-in is spent or the vector is stale");
    assert!(outcome.said.to_lowercase().contains("banana"), "{}", outcome.said);
    assert!(outcome.tool_calls > 0, "it has to have read the file rather than guessed");
    let _ = std::fs::remove_dir_all(&repo);
}
