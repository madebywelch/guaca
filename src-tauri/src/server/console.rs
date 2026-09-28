//! The operator's own shell, in an agent's terminal.
//!
//! [`crate::shell`] runs one line for a model with nothing on its stdin, which
//! is right for a model and no use to a person: `gh auth login`, `claude auth
//! login` and `codex login` ask questions, draw a menu and wait for a code, and
//! each needs a terminal to do it. An agent refused a push for credentials
//! says to run one of them, and there was nowhere in the app to run it. The
//! answer was `docker exec -it` into a container the operator may never have
//! looked at, or `ssh` to a box and then that.
//!
//! So this socket carries a real one: `bash` on a pseudo-terminal, started in
//! the agent's directory as the host's user, which is what `docker exec -it
//! <container> bash` gave and nothing more. It is behind the workspace token
//! like every other route, and it grants nothing the token did not already: a
//! connector run as a program is a command of the operator's choosing, run on
//! this host.
//!
//! ## What it is given
//!
//! The host's environment, and none of the secrets Guaca holds. A secret
//! granted to an agent reaches that agent's `shell` and its jobs and never
//! this, because this draws in the webview and a secret's value never does.
//! Their names are removed from what it inherits too, as they are for an agent
//! the secret was not granted to, so a `GH_TOKEN` the host was started with
//! cannot stand in for the sign-in being made here: `gh auth login` refuses to
//! store one while that variable is set.
//!
//! ## It lives exactly as long as the socket
//!
//! Closing the view, a network that drops and a heartbeat nobody answers all
//! end it one way. The master side of the pseudo-terminal closes, which is a
//! hangup to `bash`, which hands it on to whatever it started: how closing a
//! terminal window ends its shell everywhere else. A shell still there
//! [`GRACE`] later is killed. Nothing is kept for a client that might come
//! back, because a shell nobody is watching, holding the host's sign-ins, is
//! the one thing this must never leave behind. What was started with `nohup`
//! or `setsid` survives, because surviving is what those are for.
//!
//! ## The wire
//!
//! Bytes are binary frames in both directions and pass through untouched. A
//! character split across two reads is the terminal emulator's to put back
//! together, and a decode here would be the one place that could garble it.
//! Anything else is a text frame of JSON with a `type`: a client sends
//! `resize`, and the host sends `exit` when the shell has gone, or `refused`
//! instead of starting one, since a browser cannot read why a handshake was
//! turned away.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use pty_process::{OwnedWritePty, Size};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::domain::ids::AgentId;
use crate::runtime::Runtime;

/// The shell. `bash` because it is what the agent's own `shell` runs, so the
/// operator sees what the agent sees. Interactive because its input is a
/// terminal, so it reads the host user's `~/.bashrc`, which is what `docker
/// exec -it` did.
const SHELL: &str = "bash";

/// How long `bash` has to leave after the hangup before it is killed.
const GRACE: Duration = Duration::from_secs(2);

/// How long output is still read once `bash` has exited. What it printed last
/// can still be in the terminal's buffer, and a job it left in the background
/// can hold the terminal open forever, so this is a wait for quiet rather than
/// for the end.
const LAST_WORDS: Duration = Duration::from_millis(200);

/// How often the client is pinged, and how long it may say nothing at all.
/// The event socket's numbers, for its reason: a laptop that slept closed no
/// socket, and a shell held open for it is a shell nobody is watching.
const HEARTBEAT: Duration = Duration::from_secs(20);
const SILENCE: Duration = Duration::from_secs(60);

/// A size a client asked for, as the pseudo-terminal takes one.
pub(super) fn size(cols: u16, rows: u16) -> Size {
    let (cols, rows) = clamped(cols, rows);
    Size::new(rows, cols)
}

/// Columns and rows, clamped rather than refused. A zero is a view measured
/// before it was laid out, and a shell told it has no columns wraps every
/// character onto a line of its own; a size past any screen is a client's
/// mistake, and the terminal it asked for is still more use than none.
fn clamped(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(2, 1000), rows.clamp(1, 1000))
}

/// What the host says in a text frame.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Said {
    /// The shell has gone. `None` is a shell that was killed rather than one
    /// that exited, which is not the same as a zero.
    Exit { code: Option<i32> },
    /// No shell was started, and why, in a sentence that says what to do.
    Refused { message: String },
}

/// What a client says in a text frame.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Heard {
    Resize { cols: u16, rows: u16 },
}

/// What the loop reading the terminal is handed from the socket.
enum Key {
    Typed(Vec<u8>),
    Resized(Size),
}

/// How a shell ended.
#[derive(Debug)]
enum Ending {
    /// `bash` exited, with the code it exited with.
    Exited(Option<i32>),
    /// Nothing holds the terminal open any more, which is `bash` gone.
    Drained,
    /// The client closed, dropped, or stopped answering.
    Left,
}

/// Where a shell starts, decided before the socket is taken over.
pub(super) struct Opening {
    agent: AgentId,
    directory: PathBuf,
    /// Every secret's variable name, removed from what the shell inherits.
    hidden: Vec<String>,
}

/// Where this agent's shell would start, or the sentence saying why there is
/// none.
pub(super) fn prepare(runtime: &Runtime, agent: AgentId) -> Result<Opening, String> {
    let card = runtime
        .store()
        .get_agent(agent)
        .map_err(|err| format!("the workspace could not read that agent ({err}). Try again"))?
        .ok_or("that agent is gone, so there is no terminal to open")?;
    if !card.has_terminal {
        return Err(format!(
            "{} has no terminal. Give it one from the Terminal section of its panel first",
            card.name
        ));
    }
    let directory = runtime.terminals().ensure(agent).map_err(|err| err.to_string())?;
    let hidden = runtime
        .store()
        .connector_names()
        .map_err(|err| format!("the workspace could not read its secrets ({err}). Try again"))?;
    Ok(Opening { agent, directory, hidden })
}

/// Serves one shell on one socket until either of them is gone.
pub(super) async fn serve(mut socket: WebSocket, opening: Result<Opening, String>, window: Size) {
    let opening = match opening {
        Ok(opening) => opening,
        Err(message) => return refuse(socket, message).await,
    };
    let (pty, mut child) = match spawn(&opening, window) {
        Ok(spawned) => spawned,
        Err(err) => {
            tracing::warn!(agent = %opening.agent, %err, "the operator's shell did not start");
            let message = format!(
                "`{SHELL}` could not be started on the host ({err}), so there is no shell. Check \
                 that it is installed there"
            );
            return refuse(socket, message).await;
        }
    };
    let began = Instant::now();
    tracing::info!(agent = %opening.agent, pid = ?child.id(), "the operator opened a shell");

    let (mut output, input) = pty.into_split();
    // Unbounded, because what fills it is a person typing and pasting. Bounded,
    // a paste into a program that is not reading would stop this loop from
    // reading the output that program is waiting to write.
    let (keys, typed) = mpsc::unbounded_channel();
    let typist = tokio::spawn(type_into(input, typed));

    let mut buffer = vec![0u8; 16 * 1024];
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    let mut heard = Instant::now();
    let ending = loop {
        tokio::select! {
            read = output.read(&mut buffer) => match read {
                // Linux answers a read of a terminal nobody holds with EIO,
                // not with the end of a file. Either is the shell gone.
                Ok(0) | Err(_) => break Ending::Drained,
                Ok(n) => {
                    if socket.send(Message::Binary(buffer[..n].to_vec())).await.is_err() {
                        break Ending::Left;
                    }
                }
            },
            status = child.wait() => break Ending::Exited(status.ok().and_then(|s| s.code())),
            incoming = socket.recv() => {
                heard = Instant::now();
                match incoming {
                    Some(Ok(Message::Binary(bytes))) => {
                        let _ = keys.send(Key::Typed(bytes));
                    }
                    Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                        Ok(Heard::Resize { cols, rows }) => {
                            let _ = keys.send(Key::Resized(size(cols, rows)));
                        }
                        // A newer page saying something this host has no word
                        // for. Ignored rather than fatal, as the event socket
                        // ignores a frame it cannot parse.
                        Err(err) => tracing::debug!(%err, "an unknown console frame"),
                    },
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break Ending::Left,
                    Some(Ok(_)) => {}
                }
            }
            _ = heartbeat.tick() => {
                if heard.elapsed() > SILENCE
                    || socket.send(Message::Ping(Vec::new())).await.is_err()
                {
                    break Ending::Left;
                }
            }
        }
    };

    if matches!(ending, Ending::Exited(_)) {
        while let Ok(Ok(n)) = tokio::time::timeout(LAST_WORDS, output.read(&mut buffer)).await {
            if n == 0 || socket.send(Message::Binary(buffer[..n].to_vec())).await.is_err() {
                break;
            }
        }
    }

    // Both halves have to go for the master to close, and the hangup is what
    // ends `bash` and what it was running. The typist may be stuck writing to a
    // program that stopped reading, so it is ended rather than waited for.
    drop(keys);
    typist.abort();
    let _ = typist.await;
    drop(output);

    let code = match ending {
        Ending::Exited(code) => code,
        Ending::Drained | Ending::Left => settle(&mut child).await,
    };
    tracing::info!(
        agent = %opening.agent,
        ?ending,
        ?code,
        seconds = began.elapsed().as_secs(),
        "the operator's shell closed"
    );
    if !matches!(ending, Ending::Left) {
        let said = serde_json::to_string(&Said::Exit { code }).expect("an exit serializes");
        let _ = socket.send(Message::Text(said)).await;
        let _ = socket.close().await;
    }
}

/// Starts `bash` on a new pseudo-terminal of the given size.
fn spawn(
    opening: &Opening,
    size: Size,
) -> pty_process::Result<(pty_process::Pty, tokio::process::Child)> {
    let (pty, pts) = pty_process::open()?;
    // Before the spawn, so the first prompt is drawn at the width it will be
    // read at rather than at a default and then redrawn.
    pty.resize(size)?;
    let mut command = pty_process::Command::new(SHELL)
        .current_dir(&opening.directory)
        // What xterm.js is. A program that finds no `TERM` draws for a dumb
        // terminal, and a menu like `gh`'s is the first thing to go.
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .kill_on_drop(true);
    for name in &opening.hidden {
        command = command.env_remove(name);
    }
    let child = command.spawn(pts)?;
    Ok((pty, child))
}

/// Writes what the operator typed, in the order they typed it.
///
/// A task of its own because a write can wait: a terminal whose program is not
/// reading fills, and a write that waited inside the loop above would stop it
/// reading the output, which is the only thing that would ever empty it. A
/// resize comes through here too, so it lands between the keys it was
/// between.
async fn type_into(mut input: OwnedWritePty, mut keys: mpsc::UnboundedReceiver<Key>) {
    while let Some(key) = keys.recv().await {
        match key {
            Key::Typed(bytes) => {
                if input.write_all(&bytes).await.is_err() {
                    return;
                }
            }
            Key::Resized(size) => {
                if let Err(err) = input.resize(size) {
                    tracing::debug!(%err, "a console resize was not taken");
                }
            }
        }
    }
}

/// Waits out [`GRACE`] for a shell that was hung up on, then kills it.
async fn settle(child: &mut tokio::process::Child) -> Option<i32> {
    match tokio::time::timeout(GRACE, child.wait()).await {
        Ok(status) => status.ok().and_then(|status| status.code()),
        Err(_) => {
            tracing::warn!(pid = ?child.id(), "a shell outlived its hangup and was killed");
            let _ = child.kill().await;
            None
        }
    }
}

/// Says why there is no shell, and closes.
async fn refuse(mut socket: WebSocket, message: String) {
    let said = serde_json::to_string(&Said::Refused { message }).expect("a refusal serializes");
    let _ = socket.send(Message::Text(said)).await;
    let _ = socket.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two halves of the wire the page reads, spelled as `console.test.ts`
    // spells them. A rename on one side is a terminal that never says it ended.
    #[test]
    fn the_host_says_exit_and_refused_in_the_shape_the_page_reads() {
        assert_eq!(
            serde_json::to_value(Said::Exit { code: Some(7) }).unwrap(),
            serde_json::json!({"type": "exit", "code": 7})
        );
        assert_eq!(
            serde_json::to_value(Said::Exit { code: None }).unwrap(),
            serde_json::json!({"type": "exit", "code": null})
        );
        assert_eq!(
            serde_json::to_value(Said::Refused { message: "no".into() }).unwrap(),
            serde_json::json!({"type": "refused", "message": "no"})
        );
    }

    #[test]
    fn a_resize_is_read_as_the_page_sends_it_and_nothing_else_is() {
        assert_eq!(
            serde_json::from_str::<Heard>(r#"{"type":"resize","cols":120,"rows":40}"#).unwrap(),
            Heard::Resize { cols: 120, rows: 40 }
        );
        assert!(serde_json::from_str::<Heard>(r#"{"type":"paste","text":"x"}"#).is_err());
        assert!(serde_json::from_str::<Heard>(r#"{"type":"resize","cols":-1,"rows":40}"#).is_err());
    }

    #[test]
    fn a_size_no_screen_has_is_clamped_to_one_that_draws() {
        assert_eq!(clamped(0, 0), (2, 1));
        assert_eq!(clamped(u16::MAX, u16::MAX), (1000, 1000));
        assert_eq!(clamped(100, 30), (100, 30));
    }
}
