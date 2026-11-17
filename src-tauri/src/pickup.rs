use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const CODEX: &str = "codex";

const CODEX_PLAN: &str = "Logged in using ChatGPT";

const WITHIN: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Program {
    ClaudeCode,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pickup {
    pub program: Program,
    pub account: String,
    pub plan: String,
}

pub async fn find() -> Vec<Pickup> {
    let (claude, codex) =
        tokio::join!(from_claude(crate::llm::claude::PROGRAM), from_codex(CODEX),);
    [claude, codex].into_iter().flatten().collect()
}

pub(crate) async fn from_claude(program: &str) -> Option<Pickup> {
    let output = answer(program, &["auth", "status"]).await?;
    let said = String::from_utf8_lossy(&output.stdout);
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

async fn from_codex(program: &str) -> Option<Pickup> {
    let output = answer(program, &["login", "status"]).await?;
    if ![&output.stdout, &output.stderr].iter().any(|stream| {
        String::from_utf8_lossy(stream)
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(CODEX_PLAN))
    }) {
        return None;
    }
    Some(Pickup { program: Program::Codex, account: String::new(), plan: String::new() })
}

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
        let (_dir, at) =
            says(r#"{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty"}"#, 1);
        assert_eq!(from_claude(&at).await, None);
    }

    #[tokio::test]
    async fn a_claude_program_signed_in_with_a_key_is_not_a_plan() {
        let (_dir, at) = says(
            r#"{"loggedIn":true,"authMethod":"api_key","apiKeySource":"ANTHROPIC_API_KEY"}"#,
            0,
        );
        assert_eq!(from_claude(&at).await, None);
    }

    #[tokio::test]
    async fn a_claude_program_that_does_not_know_the_question_offers_nothing() {
        let (_dir, at) = says("Usage: claude auth <login|logout>", 0);
        assert_eq!(from_claude(&at).await, None);
    }

    #[tokio::test]
    async fn a_claude_answer_with_its_braces_out_of_order_offers_nothing() {
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
