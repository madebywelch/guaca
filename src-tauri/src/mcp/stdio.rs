//! The third transport: a server that is a program on the host, spoken to over
//! its own stdin and stdout, one JSON-RPC message per line.
//!
//! Most MCP servers people run for themselves ship this way: `npx` a package,
//! `uvx` another, hand it a token in an environment variable. A connector that
//! could only dial an address could not reach any of them without somebody
//! wrapping each in an HTTP server first.
//!
//! ## A process per session, which is a process per call
//!
//! The same rule the HTTP transports follow: nothing is kept between calls, so
//! nothing can go stale between them. For a POST that rule costs tens of
//! milliseconds. For a program it costs its start, which for `npx` is a second
//! or two with a warm cache, and that is the measurement to take before
//! changing it: a pool of long-lived servers is a second lifecycle to supervise
//! (restarts, idle timeouts, a server that wedges), and a server that keeps
//! state between calls is the one case this shape cannot serve.
//!
//! ## What the program is given
//!
//! A cleared environment, a small baseline so it can find its own programs and
//! a home directory, and the variables the operator wrote for it. Nothing else:
//! the host's own environment holds its workspace token and whatever provider
//! keys it was started with, and a server somebody found on npm has no business
//! reading either. Its working directory is the temporary directory, so a
//! server that writes beside itself does not write into the daemon's.

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::{client_info, result_of, Era, McpError, Session, Transport, CONNECT_TIMEOUT};
use super::{Dial, LEGACY_VERSION};

/// The longest single message a program may write. A tool's whole answer is one
/// line, and a large query result is megabytes; past this it is a program
/// writing without newlines, and reading on would be reading until memory runs
/// out.
const MAX_LINE: usize = 16 * 1024 * 1024;

/// How much of what a program wrote to stderr is kept, for the one message that
/// needs it: a program that died or never answered usually said why there.
const STDERR_TAIL: usize = 2_000;

/// What a program inherits from the host, when the host has it.
const BASELINE: [&str; 9] =
    ["PATH", "HOME", "USER", "LANG", "LC_ALL", "TMPDIR", "SHELL", "TERM", "XDG_CACHE_HOME"];

/// One running server. Killed when the last session holding it is dropped.
pub(crate) struct Process {
    endpoint: String,
    stdin: tokio::sync::Mutex<ChildStdin>,
    stdout: tokio::sync::Mutex<BufReader<ChildStdout>>,
    /// Held for `kill_on_drop`, which is the whole of how a process ends.
    _child: std::sync::Mutex<Child>,
    stderr: Arc<std::sync::Mutex<String>>,
    next: AtomicU64,
}

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Process").field("endpoint", &self.endpoint).finish_non_exhaustive()
    }
}

fn failed(endpoint: &str, detail: impl Into<String>) -> McpError {
    McpError::Program { endpoint: endpoint.to_string(), detail: detail.into() }
}

/// Starts the program a `stdio:` address names and shakes hands with it.
pub(super) async fn open(dial: Dial<'_>) -> Result<Session, McpError> {
    let endpoint = dial.endpoint;
    let words = crate::domain::plugin::command_line(endpoint)
        .ok_or_else(|| failed(endpoint, "the command is empty or has an unclosed quote"))?;
    let (program, args) = words.split_first().expect("command_line never returns empty");

    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(BASELINE.iter().filter_map(|name| std::env::var(name).ok().map(|v| (*name, v))))
        .envs(dial.headers.iter().map(|(name, value)| (name.as_str(), value.as_str())))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|err| {
        failed(
            endpoint,
            format!(
                "`{program}` could not be started ({err}). Check that it is installed on the host \
                 this workspace runs on, and spelled as it would be in a terminal there"
            ),
        )
    })?;

    let stdin = child.stdin.take().ok_or_else(|| failed(endpoint, "no stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| failed(endpoint, "no stdout"))?;
    let stderr = Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(mut pipe) = child.stderr.take() {
        let tail = stderr.clone();
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            while let Ok(read) = pipe.read(&mut chunk).await {
                if read == 0 {
                    break;
                }
                let mut held = tail.lock().unwrap_or_else(|e| e.into_inner());
                held.push_str(&String::from_utf8_lossy(&chunk[..read]));
                if held.len() > STDERR_TAIL {
                    let cut = held.len() - STDERR_TAIL;
                    let cut = (cut..held.len()).find(|at| held.is_char_boundary(*at)).unwrap_or(0);
                    held.drain(..cut);
                }
            }
        });
    }

    let process = Arc::new(Process {
        endpoint: endpoint.to_string(),
        stdin: tokio::sync::Mutex::new(stdin),
        stdout: tokio::sync::Mutex::new(BufReader::new(stdout)),
        _child: std::sync::Mutex::new(child),
        stderr,
        next: AtomicU64::new(1),
    });

    let answered = request(
        &process,
        "initialize",
        serde_json::json!({
            "protocolVersion": LEGACY_VERSION,
            "capabilities": {},
            "clientInfo": client_info(),
        }),
        CONNECT_TIMEOUT,
    )
    .await?;
    notify(&process, "notifications/initialized").await?;

    let negotiated = answered
        .get("protocolVersion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(LEGACY_VERSION)
        .to_string();
    let server_name = answered
        .pointer("/serverInfo/name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(Session {
        endpoint: endpoint.to_string(),
        token: None,
        headers: Vec::new(),
        era: Era::Legacy,
        transport: Transport::Stdio,
        session_id: None,
        server_name,
        negotiated: Some(negotiated),
        process: Some(process),
    })
}

/// One request, answered on the same process.
///
/// Requests are serialized per process: a session is one call, and a program
/// that interleaved two answers would be one this client would have to match up
/// by id anyway. Anything the program sends that is not the answer is handled
/// on the way past: a notification is ignored, a `ping` is answered, and any
/// other request of its own is refused so it does not wait on a client that
/// offers it nothing.
pub(super) async fn request(
    process: &Process,
    method: &str,
    params: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, McpError> {
    let id = process.next.fetch_add(1, Ordering::SeqCst);
    let message =
        serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    send(process, &message).await?;

    let answer = tokio::time::timeout(timeout, async {
        let mut stdout = process.stdout.lock().await;
        loop {
            let line = read_line(&process.endpoint, &mut stdout).await?;
            let Some(line) = line else {
                return Err(failed(
                    &process.endpoint,
                    format!("it exited before answering {method}{}", said(process)),
                ));
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                // A banner or a log line on stdout. Not MCP, and not fatal:
                // plenty of servers print one before their first message.
                tracing::debug!(endpoint = %process.endpoint, "skipping a line that is not JSON");
                continue;
            };
            if value.get("id").and_then(serde_json::Value::as_u64) == Some(id)
                && value.get("method").is_none()
            {
                return Ok(value);
            }
            if let (Some(asked), Some(their_id)) =
                (value.get("method").and_then(serde_json::Value::as_str), value.get("id"))
            {
                let reply = if asked == "ping" {
                    serde_json::json!({ "jsonrpc": "2.0", "id": their_id, "result": {} })
                } else {
                    serde_json::json!({ "jsonrpc": "2.0", "id": their_id, "error": {
                        "code": -32601, "message": "Guaca offers no client methods",
                    }})
                };
                drop(stdout);
                send(process, &reply).await?;
                stdout = process.stdout.lock().await;
            }
        }
    })
    .await
    .map_err(|_| {
        failed(
            &process.endpoint,
            format!("it did not answer {method} within {}s{}", timeout.as_secs(), said(process)),
        )
    })??;
    result_of(&process.endpoint, &answer)
}

async fn notify(process: &Process, method: &str) -> Result<(), McpError> {
    send(process, &serde_json::json!({ "jsonrpc": "2.0", "method": method })).await
}

async fn send(process: &Process, message: &serde_json::Value) -> Result<(), McpError> {
    let mut line = serde_json::to_vec(message).expect("a JSON value serializes");
    line.push(b'\n');
    let mut stdin = process.stdin.lock().await;
    stdin.write_all(&line).await.map_err(|err| {
        failed(&process.endpoint, format!("it stopped reading its input ({err}){}", said(process)))
    })?;
    stdin.flush().await.map_err(|err| failed(&process.endpoint, err.to_string()))
}

/// One line, or `None` at the end of the stream, capped at [`MAX_LINE`].
async fn read_line(
    endpoint: &str,
    stdout: &mut BufReader<ChildStdout>,
) -> Result<Option<String>, McpError> {
    let mut buf = Vec::new();
    let read = (&mut *stdout)
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut buf)
        .await
        .map_err(|err| failed(endpoint, format!("its output could not be read ({err})")))?;
    if read == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_LINE {
        return Err(failed(endpoint, "it wrote a message longer than Guaca reads"));
    }
    Ok(Some(String::from_utf8_lossy(&buf).trim().to_string()))
}

/// What the program last said on stderr, as the end of a sentence.
fn said(process: &Process) -> String {
    let tail = process.stderr.lock().map(|held| held.trim().to_string()).unwrap_or_default();
    if tail.is_empty() {
        String::new()
    } else {
        format!(". It said: {tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{call_tool, list_tools, open, Dial, McpError};

    fn fixture(mode: &str) -> String {
        format!("stdio:python3 {}/tests/fixtures/mcp-stdio.py {mode}", env!("CARGO_MANIFEST_DIR"))
    }

    #[tokio::test]
    async fn a_program_is_started_asked_and_answered_over_its_own_pipes() {
        let endpoint = fixture("");
        let env = [("FIXTURE_TOKEN".to_string(), "abc".to_string())];
        let dial = Dial { endpoint: &endpoint, token: None, headers: &env, legacy_transport: true };
        let session = open(dial).await.expect("the program starts and shakes hands");
        assert!(session.stdio());
        assert_eq!(session.server_name, "fixture");
        assert_eq!(session.protocol(), "2025-06-18");

        let tools = list_tools(&session).await.unwrap();
        assert_eq!(
            tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["echo", "whoami", "fail"]
        );

        let echoed = call_tool(&session, "echo", &serde_json::json!({ "text": "hi" }), None).await;
        assert_eq!(echoed.unwrap().text, "said: hi");

        // The operator's variable arrives; the host's own environment does
        // not, and a question the server asks mid-call is refused, not ignored.
        // Cargo puts this in the test's environment, which is what makes its
        // absence in the program's a finding.
        assert!(std::env::var("CARGO_PKG_NAME").is_ok());
        let said = call_tool(&session, "whoami", &serde_json::json!({}), None).await.unwrap();
        assert_eq!(said.text, "token=abc leaked=False refused=True");

        let refused = call_tool(&session, "fail", &serde_json::json!({}), None).await;
        assert!(
            matches!(refused, Err(McpError::Rejected { ref message }) if message == "fail always fails")
        );
    }

    #[tokio::test]
    async fn a_program_that_cannot_run_says_why_in_its_own_words() {
        let missing = "stdio:guaca-no-such-program --flag";
        let err = open(Dial::to(missing)).await.unwrap_err().to_string();
        assert!(err.contains("could not be started"), "{err}");
        assert!(err.contains("installed on the host"), "{err}");

        let dies = fixture("die");
        let err = open(Dial::to(&dies)).await.unwrap_err().to_string();
        assert!(err.contains("exited before answering initialize"), "{err}");
        assert!(err.contains("cannot find its configuration"), "the stderr tail is quoted: {err}");
    }
}
