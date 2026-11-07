//! A host's own Claude CLI sign-in. The CLI keeps the credential; this module
//! relays only its authorization URL and the browser's one-time code.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, watch, Mutex};
use tokio::task::JoinHandle;

const EXPIRES: Duration = Duration::from_secs(15 * 60);
const STARTS_WITHIN: Duration = Duration::from_secs(30);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
}

#[derive(Serialize)]
pub struct Started {
    pub id: String,
    pub url: String,
}

struct Session {
    code: Option<oneshot::Sender<String>>,
    result: Option<oneshot::Receiver<bool>>,
    cancel: watch::Sender<bool>,
    task: JoinHandle<()>,
}

struct Starting(Option<watch::Sender<bool>>);

impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            let _ = cancel.send(true);
        }
    }
}

#[derive(Default)]
pub struct Signin {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}

impl Signin {
    pub async fn status(program: &str) -> Status {
        Status { signed_in: crate::pickup::from_claude(program).await.is_some() }
    }

    pub async fn begin(&self, program: &str) -> Result<Started, &'static str> {
        let id = uuid::Uuid::new_v4().to_string();
        let (url_tx, url_rx) = oneshot::channel();
        let (code_tx, code_rx) = oneshot::channel();
        let (result_tx, result_rx) = oneshot::channel();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let mut starting = {
            let mut sessions = self.sessions.lock().await;
            for session in sessions.values_mut() {
                let _ = session.cancel.send(true);
                let _ = (&mut session.task).await;
            }
            sessions.clear();
            let program = program.to_owned();
            let task = tokio::spawn(async move {
                let ok = run(&program, url_tx, code_rx, cancel_rx).await;
                let _ = result_tx.send(ok);
            });
            let starting = Starting(Some(cancel_tx.clone()));
            sessions.insert(
                id.clone(),
                Session { code: Some(code_tx), result: Some(result_rx), cancel: cancel_tx, task },
            );
            starting
        };
        match tokio::time::timeout(STARTS_WITHIN, url_rx).await {
            Ok(Ok(url)) => {
                starting.0 = None;
                Ok(Started { id, url })
            }
            _ => {
                self.cancel(&id).await;
                Err("Claude could not start sign-in. Try again.")
            }
        }
    }

    pub async fn complete(&self, id: &str, code: String) -> Result<Status, &'static str> {
        if code.is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
            return Err("Enter the code from the Claude sign-in page.");
        }
        let result = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.get_mut(id).ok_or("This sign-in has expired. Start again.")?;
            let sender = session.code.take().ok_or("This sign-in is already being completed.")?;
            let result = session.result.take().ok_or("This sign-in is already being completed.")?;
            sender.send(code).map_err(|_| "This sign-in has expired. Start again.")?;
            result
        };
        let signed_in = result.await.unwrap_or(false);
        self.cancel(id).await;
        if signed_in {
            Ok(Status { signed_in: true })
        } else {
            Err("Claude could not complete sign-in. Try again.")
        }
    }

    pub async fn cancel(&self, id: &str) {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(id) {
            let _ = session.cancel.send(true);
            let _ = (&mut session.task).await;
        }
        sessions.remove(id);
    }
}

async fn run(
    program: &str,
    url_tx: oneshot::Sender<String>,
    code_rx: oneshot::Receiver<String>,
    mut cancel: watch::Receiver<bool>,
) -> bool {
    let Ok(mut child) = tokio::process::Command::new(program)
        .args(["auth", "login", "--claudeai"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    else {
        return false;
    };
    let result = tokio::select! {
        result = perform(program, &mut child, url_tx, code_rx) => result,
        _ = cancel.changed() => false,
        _ = tokio::time::sleep(EXPIRES) => false,
    };
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
    result
}

async fn perform(
    program: &str,
    child: &mut tokio::process::Child,
    url_tx: oneshot::Sender<String>,
    code_rx: oneshot::Receiver<String>,
) -> bool {
    let Some(stdout) = child.stdout.take() else { return false };
    let Some(mut stdin) = child.stdin.take() else { return false };
    let mut lines = BufReader::new(stdout);
    let mut line = Vec::new();
    let url = loop {
        line.clear();
        match (&mut lines).take(8193).read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return false,
            Ok(_) if line.len() > 8192 => return false,
            Ok(_) => {}
        }
        let Ok(text) = std::str::from_utf8(&line) else { return false };
        if let Some(url) = text.trim().strip_prefix("If the browser didn't open, visit: ") {
            let Ok(parsed) = reqwest::Url::parse(url) else { return false };
            if parsed.scheme() != "https"
                || parsed.host_str() != Some("claude.com")
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.port().is_some()
                || parsed.fragment().is_some()
                || parsed.path() != "/cai/oauth/authorize"
            {
                return false;
            }
            break url.to_owned();
        }
    };
    if url_tx.send(url).is_err() {
        return false;
    }
    let Ok(code) = code_rx.await else { return false };
    if stdin.write_all(code.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() {
        return false;
    }
    drop(stdin);
    let mut sink = tokio::io::sink();
    let (exit, _) = tokio::join!(child.wait(), tokio::io::copy(&mut lines, &mut sink));
    let success = exit.is_ok_and(|status| status.success());
    success && Signin::status(program).await.signed_in
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn program(body: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, path.to_str().unwrap().to_owned())
    }

    const PROMPTS: &str = "printf \"Opening browser to sign in…\\nIf the browser didn't open, visit: https://claude.com/cai/oauth/authorize?state=test\\nPaste code here if prompted > \"";

    #[tokio::test]
    async fn the_program_receives_the_browser_code_and_confirms_its_own_status() {
        let (_dir, path) = program(&format!(
            "if [ \"$2\" = status ]; then printf '%s' '{{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}}'; exit 0; fi\n{PROMPTS}\nread code\n[ \"$code\" = 'one-time-code' ]"
        ));
        let signin = Signin::default();
        let started = signin.begin(&path).await.unwrap();
        assert_eq!(started.url, "https://claude.com/cai/oauth/authorize?state=test");
        assert!(signin.complete(&started.id, "one-time-code".into()).await.unwrap().signed_in);
    }

    #[tokio::test]
    async fn a_successful_exit_is_not_enough_without_a_subscription_status() {
        let (_dir, path) = program(&format!(
            "if [ \"$2\" = status ]; then printf '%s' '{{\"loggedIn\":true,\"authMethod\":\"api_key\"}}'; exit 0; fi\n{PROMPTS}\nread code\nexit 0"
        ));
        let signin = Signin::default();
        let started = signin.begin(&path).await.unwrap();
        assert!(signin.complete(&started.id, "code".into()).await.is_err());
    }

    #[tokio::test]
    async fn cancelled_sessions_cannot_be_completed_and_release_the_slot() {
        let (_dir, path) = program(&format!("{PROMPTS}\nread code\nexit 1"));
        let signin = Signin::default();
        let started = signin.begin(&path).await.unwrap();
        let replacement = signin.begin(&path).await.unwrap();
        assert!(signin.complete(&started.id, "code".into()).await.is_err());
        signin.cancel(&replacement.id).await;
        let next = signin.begin(&path).await.unwrap();
        assert!(signin.complete(&next.id, "code".into()).await.is_err());
    }

    #[tokio::test]
    async fn a_program_cannot_send_an_untrusted_signin_url() {
        let (_dir, path) =
            program("printf \"If the browser didn't open, visit: https://example.com/steal\\n\"");
        assert!(Signin::default().begin(&path).await.is_err());
    }

    #[tokio::test]
    async fn an_unterminated_long_line_is_refused_without_waiting_for_exit() {
        let (_dir, path) = program("head -c 9000 /dev/zero | tr '\\0' x\nsleep 60");
        let result =
            tokio::time::timeout(Duration::from_secs(5), Signin::default().begin(&path)).await;
        assert!(result.unwrap().is_err());
    }

    #[tokio::test]
    async fn a_submitted_code_can_still_be_cancelled() {
        let (_dir, path) = program(&format!("{PROMPTS}\nread code\nexec sleep 60"));
        let signin = Arc::new(Signin::default());
        let started = signin.begin(&path).await.unwrap();
        let other = signin.clone();
        let id = started.id.clone();
        let pending = tokio::spawn(async move { other.complete(&id, "code".into()).await });
        tokio::task::yield_now().await;
        signin.cancel(&started.id).await;
        assert!(tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
}
