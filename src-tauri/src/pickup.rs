//! A sign-in this machine already has, offered once on first run.
//!
//! Two programs can hold one. The `claude` program is the one this app already
//! runs for the Claude provider, and the Codex CLI is the terminal login for the
//! ChatGPT plan that pays for the other subscription.
//!
//! ## Detection is an offer, never a pickup
//!
//! Nothing here changes a setting. It asks two programs a question about
//! themselves so the first run can say what it found and let the operator
//! choose. A workspace that adopted a credential because it happened to be on
//! the disk would be spending an account its operator never named, and on a
//! machine somebody else also uses that account may not even be theirs.
//!
//! ## The Claude credential is never read
//!
//! `claude auth status` is the program's own answer: a JSON document and an exit
//! code. Guaca does not open `~/.claude`, does not ask the Keychain, and could
//! not use a token if it had one. `docs/PROTOCOL.md` says why the plan can only
//! be spent by being the program, and `llm/claude.rs` is the whole of that
//! arrangement; this file asks whether the arrangement is ready.
//!
//! Only the two OAuth methods count. `api_key` and `api_key_helper` mean the
//! program would spend an Anthropic key rather than a plan, and `third_party`
//! means somebody's gateway. Offering either as "your Claude subscription" would
//! name the wrong thing and then bill the wrong account.
//!
//! ## The Codex token is deliberately not imported
//!
//! `~/.codex/auth.json` holds a refresh token, and a refresh token rotates:
//! whichever program spends it next retires the copy every other holder has.
//! That is already true inside this app, which is why `Subscription::renew` is
//! serialized (`docs/ARCHITECTURE.md`, *A token's `exp` is a floor on its life,
//! not a ceiling*). Across two processes there is no lock to serialize, so
//! copying the CLI's token would leave the CLI and Guaca signing each other out
//! at the next refresh, and the operator would see two programs alternately
//! broken with no way to tell which one did it.
//!
//! So detection stops at "this Mac is signed in to Codex", and what is offered
//! is Guaca's own sign-in, through the browser session that CLI already opened.
//! The panel says why it is asking rather than leaving the operator to wonder.
//!
//! `codex login status` prints the mode and exits 0 when it is signed in, and
//! the mode is the only signal there is: the CLI names the account nowhere. An
//! API-key login is a different thing from a plan and is refused here for the
//! same reason the Claude key is.

use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The Codex CLI, found on `PATH` for the same reason the `claude` program is
/// ([`crate::llm::claude::PROGRAM`]): a second place to say where it lives is a
/// second place for it to be wrong.
pub const CODEX: &str = "codex";

/// What `codex login status` says when the plan is what pays.
const CODEX_PLAN: &str = "Logged in using ChatGPT";

/// How long a program gets to say who is signed in.
///
/// Both read one local file and print one line. A program that needs longer is
/// one that will not answer at all, and a first run must not sit on it: the
/// offer is a shortcut, so a slow question is a question worth dropping.
const WITHIN: Duration = Duration::from_secs(10);

/// Which program answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Program {
    ClaudeCode,
    Codex,
}

/// A sign-in found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pickup {
    /// Which program it is, which is also which card the panel draws.
    pub program: Program,
    /// The account the program named, or empty when it names none.
    ///
    /// An address, never a token. `codex login status` names no account, so its
    /// pickup carries an empty one rather than a guess read out of the token.
    pub account: String,
    /// The plan the program reported, or empty.
    pub plan: String,
}

/// Every sign-in this machine has that Guaca could offer.
///
/// Both programs are asked at once because either can take a second to start,
/// and neither answer depends on the other. An absent program, a refused one and
/// a silent one are the same thing here: nothing to offer.
pub async fn find() -> Vec<Pickup> {
    let (claude, codex) =
        tokio::join!(from_claude(crate::llm::claude::PROGRAM), from_codex(CODEX),);
    [claude, codex].into_iter().flatten().collect()
}

/// What the `claude` program says about its own sign-in.
///
/// The program is named rather than read from a constant so the suite can hand
/// it a stand-in: the alternative is a test that sets `PATH` for the whole
/// process, which races every other test in the binary.
pub(crate) async fn from_claude(program: &str) -> Option<Pickup> {
    let output = answer(program, &["auth", "status"]).await?;
    let said = String::from_utf8_lossy(&output.stdout);
    // A banner before the document is why this is a slice rather than the whole
    // string. Truncated output, a version that does not know the question and a
    // program that never ran all end here, as nothing to offer. The closing
    // brace is searched for after the opening one, so output that has them the
    // other way round is unparseable rather than a panic in a command handler.
    let document = &said[said.find('{')?..];
    let document = &document[..=document.rfind('}')?];
    let status: ClaudeStatus = serde_json::from_str(document).ok()?;
    if !status.logged_in || !matches!(status.auth_method.as_str(), "claude.ai" | "oauth_token") {
        return None;
    }
    Some(Pickup {
        program: Program::ClaudeCode,
        account: status.email.unwrap_or_default(),
        plan: status.subscription_type.unwrap_or_default(),
    })
}

/// What the Codex CLI says about its own sign-in.
async fn from_codex(program: &str) -> Option<Pickup> {
    let output = answer(program, &["login", "status"]).await?;
    // A line rather than the whole document, because the CLI prints one line
    // and a banner above it would otherwise hide the answer.
    if ![&output.stdout, &output.stderr].iter().any(|stream| {
        String::from_utf8_lossy(stream)
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(CODEX_PLAN))
    }) {
        return None;
    }
    Some(Pickup { program: Program::Codex, account: String::new(), plan: String::new() })
}

/// Runs one question and returns what it wrote, when it says it worked.
///
/// A non-zero exit is the answer to "is it there and signed in", so it is
/// `None`, not an error. Nothing here is logged or carried into a sentence,
/// because a program's own failure text has been known to quote what it read.
async fn answer(program: &str, args: &[&str]) -> Option<std::process::Output> {
    let asked = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let answered = tokio::time::timeout(WITHIN, asked).await.ok()?.ok()?;
    answered.status.success().then_some(answered)
}

/// The fields `claude auth status` writes that this file reads.
///
/// Everything else it reports is personal (`orgId`, `orgName`) or about the
/// program's configuration (`apiProvider`), and none of it changes the offer.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeStatus {
    #[serde(default)]
    logged_in: bool,
    #[serde(default)]
    auth_method: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    subscription_type: Option<String>,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A stand-in program that prints one line and exits with a code.
    ///
    /// A shell script rather than a compiled fixture: the thing under test is
    /// how an answer is read, and a script is the smallest thing that can give
    /// one. It lives in its own directory so two tests cannot see each other's.
    fn says(text: &str, exit: u8) -> (tempfile::TempDir, String) {
        assert!(!text.contains('\''), "the fixture writes its text in single quotes");
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("program");
        crate::testing::write_executable(
            &at,
            &format!("#!/bin/sh\nprintf '%s' '{text}'\nexit {exit}\n"),
        );
        let path = at.to_string_lossy().into_owned();
        (dir, path)
    }

    #[tokio::test]
    async fn a_signed_in_claude_plan_is_offered_with_its_account() {
        let (_dir, at) = says(
            r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"robin@example.com","subscriptionType":"max"}"#,
            0,
        );
        let found = from_claude(&at).await.expect("a signed-in plan is a pickup");
        assert_eq!(found.program, Program::ClaudeCode);
        assert_eq!(found.account, "robin@example.com");
        assert_eq!(found.plan, "max");
    }

    #[tokio::test]
    async fn a_claude_program_logged_out_offers_nothing() {
        // The CLI exits 1 when it is not signed in, which is the whole answer.
        let (_dir, at) =
            says(r#"{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty"}"#, 1);
        assert_eq!(from_claude(&at).await, None);
    }

    #[tokio::test]
    async fn a_claude_program_signed_in_with_a_key_is_not_a_plan() {
        // A key is billed per token by Anthropic and is the other provider's
        // business; calling it a Claude subscription would name the wrong thing.
        let (_dir, at) = says(
            r#"{"loggedIn":true,"authMethod":"api_key","apiKeySource":"ANTHROPIC_API_KEY"}"#,
            0,
        );
        assert_eq!(from_claude(&at).await, None);
    }

    #[tokio::test]
    async fn a_claude_program_that_does_not_know_the_question_offers_nothing() {
        // Older builds have no `auth status`. Help text and a successful exit is
        // the shape that would otherwise be read as an answer.
        let (_dir, at) = says("Usage: claude auth <login|logout>", 0);
        assert_eq!(from_claude(&at).await, None);
    }

    #[tokio::test]
    async fn a_claude_answer_with_its_braces_out_of_order_offers_nothing() {
        // Whatever a program prints, a command handler may not panic on it.
        let (_dir, at) = says("}} {{", 0);
        assert_eq!(from_claude(&at).await, None);
    }

    #[tokio::test]
    async fn a_claude_program_that_is_not_installed_offers_nothing() {
        assert_eq!(from_claude("guaca-no-such-program").await, None);
    }

    #[tokio::test]
    async fn a_codex_cli_signed_in_with_chatgpt_is_offered() {
        let (_dir, at) = says("Logged in using ChatGPT", 0);
        let found = from_codex(&at).await.expect("a ChatGPT login is a pickup");
        assert_eq!(found.program, Program::Codex);
        // The CLI names no account, and this file does not read the token to
        // find one.
        assert_eq!(found.account, "");
    }

    #[tokio::test]
    async fn a_codex_cli_that_writes_its_login_to_stderr_is_offered() {
        let dir = tempfile::tempdir().unwrap();
        let at = dir.path().join("codex");
        crate::testing::write_executable(
            &at,
            "#!/bin/sh\nprintf 'Logged in using ChatGPT\\n' >&2\nexit 0\n",
        );
        let found = from_codex(at.to_str().unwrap()).await.expect("a ChatGPT login is a pickup");
        assert_eq!(found.program, Program::Codex);
        assert_eq!(found.account, "");
    }

    #[tokio::test]
    async fn a_failed_codex_command_cannot_offer_a_login() {
        let (_dir, at) = says("Logged in using ChatGPT", 1);
        assert_eq!(from_codex(&at).await, None);
    }

    #[tokio::test]
    async fn a_codex_cli_signed_in_with_a_key_is_not_a_plan() {
        let (_dir, at) = says("Logged in using an API key - sk-proj-...", 0);
        assert_eq!(from_codex(&at).await, None);
    }

    #[tokio::test]
    async fn a_codex_cli_logged_out_offers_nothing() {
        let (_dir, at) = says("Not logged in", 1);
        assert_eq!(from_codex(&at).await, None);
    }

    #[tokio::test]
    async fn a_codex_cli_that_is_not_installed_offers_nothing() {
        assert_eq!(from_codex("guaca-no-such-program").await, None);
    }
}
