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
    state: Attempt,
    cancel: watch::Sender<bool>,
    task: JoinHandle<()>,
}

enum Attempt {
    Starting { code: oneshot::Sender<String>, result: oneshot::Receiver<Outcome> },
    AwaitingCode { code: oneshot::Sender<String>, result: oneshot::Receiver<Outcome> },
    Completing,
}

enum Outcome {
    SignedIn,
    Failed,
    InvalidCode,
    Cancelled,
    Expired,
}

struct CancelOnDrop(Option<watch::Sender<bool>>);

impl Drop for CancelOnDrop {
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
                let outcome = run(&program, url_tx, code_rx, cancel_rx, EXPIRES).await;
                let _ = result_tx.send(outcome);
            });
            let starting = CancelOnDrop(Some(cancel_tx.clone()));
            sessions.insert(
                id.clone(),
                Session {
                    state: Attempt::Starting { code: code_tx, result: result_rx },
                    cancel: cancel_tx,
                    task,
                },
            );
            starting
        };
        match tokio::time::timeout(STARTS_WITHIN, url_rx).await {
            Ok(Ok(url)) => {
                let mut sessions = self.sessions.lock().await;
                if let Some(session) = sessions.get_mut(&id) {
                    let state = std::mem::replace(&mut session.state, Attempt::Completing);
                    session.state = match state {
                        Attempt::Starting { code, result } => {
                            Attempt::AwaitingCode { code, result }
                        }
                        other => other,
                    };
                }
                starting.0 = None;
                Ok(Started { id, url })
            }
            Ok(Err(_)) => {
                self.cancel(&id).await;
                Err("Claude sign-in didn't finish.")
            }
            Err(_) => {
                self.cancel(&id).await;
                Err("Claude sign-in timed out.")
            }
        }
    }

    pub async fn complete(&self, id: &str, code: String) -> Result<Status, &'static str> {
        if code.is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
            return Err("Enter the code from the Claude sign-in page.");
        }
        let (result, mut completing) = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions.get_mut(id).ok_or("Claude sign-in is no longer active.")?;
            let state = std::mem::replace(&mut session.state, Attempt::Completing);
            let (sender, result) = match state {
                Attempt::Starting { code, result } | Attempt::AwaitingCode { code, result } => {
                    (code, result)
                }
                Attempt::Completing => return Err("This sign-in is already being completed."),
            };
            let _ = sender.send(code);
            (result, CancelOnDrop(Some(session.cancel.clone())))
        };
        let outcome = result.await.unwrap_or(Outcome::Failed);
        self.cancel(id).await;
        completing.0 = None;
        match outcome {
            Outcome::SignedIn => Ok(Status { signed_in: true }),
            Outcome::Cancelled => Err("Claude sign-in was cancelled."),
            Outcome::Expired => Err("Claude sign-in timed out."),
            Outcome::InvalidCode => Err("Claude couldn't read that code; copy the full code."),
            Outcome::Failed => Err("Claude sign-in didn't finish."),
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
    expires: Duration,
) -> Outcome {
    let Ok(mut child) = tokio::process::Command::new(program)
        .args(["auth", "login", "--claudeai"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    else {
        return Outcome::Failed;
    };
    let Some(stderr) = child.stderr.take() else { return Outcome::Failed };
    let result = tokio::select! {
        result = perform(program, &mut child, url_tx, code_rx) => if result { Outcome::SignedIn } else { Outcome::Failed },
        _ = invalid_code(stderr) => Outcome::InvalidCode,
        _ = cancel.changed() => Outcome::Cancelled,
        _ = tokio::time::sleep(expires) => Outcome::Expired,
    };
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
    result
}

async fn invalid_code(stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr);
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut lines).take(8193).read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => std::future::pending::<()>().await,
            Ok(_) if line.len() > 8192 => std::future::pending::<()>().await,
            Ok(_) => {}
        }
        if line.strip_suffix(b"\n").unwrap_or(&line)
            == b"Invalid code. Please make sure the full code was copied."
        {
            return;
        }
    }
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
    async fn the_clis_invalid_code_message_is_reported_without_its_other_output() {
        let (_dir, path) = program(&format!(
            "{PROMPTS}\nread code\nprintf '%s\\n' 'Invalid code. Please make sure the full code was copied.' >&2\nexec sleep 60"
        ));
        let signin = Signin::default();
        let started = signin.begin(&path).await.unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            signin.complete(&started.id, "dummy".into()),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err("Claude couldn't read that code; copy the full code.")));
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

    #[tokio::test]
    async fn a_second_submission_cannot_take_over_a_completing_attempt() {
        let (dir, path) =
            program(&format!("{PROMPTS}\nread code\necho ready > \"$0.ready\"\nexec sleep 60"));
        let signin = Arc::new(Signin::default());
        let started = signin.begin(&path).await.unwrap();
        let other = signin.clone();
        let id = started.id.clone();
        let pending = tokio::spawn(async move { other.complete(&id, "first".into()).await });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !dir.path().join("claude.ready").exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            signin.complete(&started.id, "second".into()).await,
            Err("This sign-in is already being completed.")
        ));
        signin.cancel(&started.id).await;
        assert!(pending.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn an_expired_attempt_returns_timeout_and_reaps_the_child() {
        let (_dir, path) = program(&format!("{PROMPTS}\nexec sleep 60"));
        let (url_tx, url_rx) = oneshot::channel();
        let (_code_tx, code_rx) = oneshot::channel();
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            run(&path, url_tx, code_rx, cancel_rx, Duration::from_secs(1)).await
        });
        url_rx.await.unwrap();
        assert!(matches!(task.await.unwrap(), Outcome::Expired));
    }

    #[tokio::test]
    async fn abandoning_completion_stops_and_reaps_the_child() {
        let (dir, path) =
            program(&format!("{PROMPTS}\nread code\necho $$ > \"$0.pid\"\nexec sleep 60"));
        let signin = Arc::new(Signin::default());
        let started = signin.begin(&path).await.unwrap();
        let other = signin.clone();
        let id = started.id.clone();
        let pending = tokio::spawn(async move { other.complete(&id, "code".into()).await });
        let pid_path = dir.path().join("claude.pid");
        let pid: u32 = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&pid_path) {
                    if let Ok(pid) = text.trim().parse() {
                        break pid;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        pending.abort();
        let _ = pending.await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the child was not reaped after completion was abandoned");
    }
}
