//! The probe: a control channel into a test build of the app, and the shape of
//! what crosses it.
//!
//! Most of what is Mac-only in Guaca happens where no test reaches: what the
//! window shows, whether macOS lets a banner through, where the menu bar panel
//! stands and who has the keyboard when it closes. The probe lets a script ask
//! the running app those questions and act in it the way a person would, so a
//! scenario can be run by a machine, three times, and come out the same.
//!
//! This file is the half with no Tauri in it: the requests, the answers, and
//! the checks on who may connect. It is compiled into the unit tests on every
//! platform, so the gate argues it on Linux too. The half that opens the
//! socket is [`channel`], and it exists only in a build made with the `probe`
//! feature, which refuses to compile with optimizations on. A shipped app has
//! neither. `docs/PROBE.md` is how the pieces fit, and `scripts/probe.py` is
//! the command that drives them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "probe")]
pub mod channel;

/// Where the channel listens. Unset, a probe build opens nothing, so the
/// socket exists only for a launch that asked for it.
pub const SOCKET_ENV: &str = "GUACA_PROBE_SOCKET";

/// One request, one line of JSON, carrying the id its answer echoes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Envelope {
    pub id: u64,
    #[serde(flatten)]
    pub request: Request,
}

/// Everything a client can ask. Each one is either a reading of state the app
/// already holds or something a person could do with a pointer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Request {
    /// Which process this is and what it calls itself. The first thing a
    /// client asks, and what it waits on after a launch.
    Hello,
    /// Windows, the notification permission as the app reads it, and the
    /// menu bar.
    State,
    /// A call into the page a window draws, answered by the page's bridge
    /// (`src/lib/probe.ts`). The channel carries it without reading it.
    Page { window: String, call: Value },
    /// A left click on the menu bar icon, through the same path a click takes.
    ClickIcon,
    /// A window's close button: what the app does with that is its own
    /// business, and for the main window it is to hide rather than quit.
    Close { window: String },
    /// The app hidden, which hands the keyboard to whatever is next in line.
    Hide,
    /// The app quits, as Quit Guaca in the icon's menu would.
    Quit,
    /// The main thread kept busy for `ms` milliseconds, in slices short
    /// enough that the window still answers, so its run loop is almost never
    /// idle: what a window drawing a long reply does to it. Answered at once.
    Busy { ms: u64 },
}

/// The answer to one request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn to(id: u64, outcome: Result<Value, String>) -> Self {
        match outcome {
            Ok(value) => Self { id, ok: true, value: Some(value), error: None },
            Err(error) => Self { id, ok: false, value: None, error: Some(error) },
        }
    }

    /// One line, newline included, ready to write.
    pub fn line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("a reply serializes");
        line.push('\n');
        line
    }
}

/// Reads one line into a request, or the answer that says why it is not one.
///
/// A line with no readable id is answered with id 0, which no client sends,
/// so a client that sent garbage still hears that it did.
pub fn parse(line: &str) -> Result<Envelope, Reply> {
    serde_json::from_str(line).map_err(|err| {
        let id = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|value| value.get("id")?.as_u64())
            .unwrap_or(0);
        Reply::to(id, Err(format!("not a probe request: {err}")))
    })
}

/// What the app reports about itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub identifier: String,
    pub pid: u32,
    pub windows: Vec<Window>,
    /// `notifications::Permission` as the app reads it, or null where the
    /// platform has no per-app answer.
    pub notifications: Value,
    /// Null when the app runs without a menu bar icon.
    pub menubar: Option<Menubar>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub label: String,
    pub visible: bool,
    pub focused: bool,
    pub minimized: bool,
}

/// The icon as it is drawn, and the panel under it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Menubar {
    pub glyph: String,
    pub title: Option<String>,
    pub tooltip: String,
    /// Whether the window has said which host the panel is to show, and the
    /// panel has loaded it. Until then a click opens the window instead.
    pub attached: bool,
    pub panel: bool,
    pub panel_visible: bool,
}

/// The script a page is asked to run, and how to read what comes back.
///
/// The page answers a call synchronously and as a JSON string, because the
/// webview hands back the value of the expression it evaluated and nothing
/// that arrives later. A page without the bridge answers that it has none,
/// rather than throwing, so a client waiting for the page to load can tell
/// "not yet" from "broken".
pub fn page_script(call: &Value) -> String {
    format!(
        "(function(){{var p=window.__guacaProbe;return JSON.stringify(p?p.run({call}):{{ok:false,error:'no probe bridge on this page'}});}})()"
    )
}

/// The page's answer, from what the webview reported.
///
/// The webview serializes the expression's value as JSON, and the value is
/// itself a JSON string, so the answer is decoded twice. The bridge's own
/// `{ ok, value | error }` becomes the channel's.
pub fn page_answer(reported: &str) -> Result<Value, String> {
    let inner: String = serde_json::from_str(reported)
        .map_err(|err| format!("the page answered something unreadable: {err}"))?;
    let answer: Value = serde_json::from_str(&inner)
        .map_err(|err| format!("the page answered something unreadable: {err}"))?;
    match answer.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(answer.get("value").cloned().unwrap_or(Value::Null)),
        Some(false) => Err(answer
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the page refused without saying why")
            .to_string()),
        None => Err("the page answered without saying whether it worked".into()),
    }
}

/// Whether a connection may talk to the channel: only the account the app
/// runs as. The socket's directory already shuts everyone else out, and this
/// is the same rule asked of the kernel a second time.
pub fn admits(peer: u32, owner: u32) -> bool {
    peer == owner
}

/// Whether a directory is fit to hold the socket: a directory, owned by the
/// account the app runs as, that nobody else can enter or list.
#[cfg(unix)]
pub fn private_dir(path: &std::path::Path, owner: u32) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta =
        std::fs::symlink_metadata(path).map_err(|err| format!("{}: {err}", path.display()))?;
    if !meta.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    if meta.uid() != owner {
        return Err(format!("{} belongs to another account", path.display()));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{} is open to other accounts; the probe needs a directory only its owner can enter",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_request_reads_from_the_line_a_client_writes() {
        let cases = [
            (r#"{"id":1,"op":"hello"}"#, Request::Hello),
            (r#"{"id":2,"op":"state"}"#, Request::State),
            (
                r#"{"id":3,"op":"page","window":"main","call":{"do":"text"}}"#,
                Request::Page { window: "main".into(), call: json!({ "do": "text" }) },
            ),
            (r#"{"id":4,"op":"clickIcon"}"#, Request::ClickIcon),
            (r#"{"id":7,"op":"close","window":"main"}"#, Request::Close { window: "main".into() }),
            (r#"{"id":5,"op":"hide"}"#, Request::Hide),
            (r#"{"id":6,"op":"quit"}"#, Request::Quit),
            (r#"{"id":8,"op":"busy","ms":5000}"#, Request::Busy { ms: 5000 }),
        ];
        for (line, request) in cases {
            let envelope = parse(line).unwrap_or_else(|reply| panic!("{line}: {reply:?}"));
            assert_eq!(envelope.request, request, "{line}");
        }
    }

    #[test]
    fn a_line_that_is_not_a_request_is_answered_with_why() {
        let reply = parse(r#"{"id":9,"op":"format the disk"}"#).unwrap_err();
        assert_eq!(reply.id, 9);
        assert!(!reply.ok);
        assert!(reply.error.unwrap().starts_with("not a probe request"));

        let reply = parse("not json").unwrap_err();
        assert_eq!(reply.id, 0);
    }

    #[test]
    fn an_answer_is_one_line_with_either_a_value_or_an_error() {
        let ok = Reply::to(3, Ok(json!({ "pid": 7 }))).line();
        assert_eq!(ok, "{\"id\":3,\"ok\":true,\"value\":{\"pid\":7}}\n");
        let refused = Reply::to(4, Err("no such window".into())).line();
        assert_eq!(refused, "{\"id\":4,\"ok\":false,\"error\":\"no such window\"}\n");
    }

    #[test]
    fn a_page_answer_is_decoded_twice_and_keeps_the_pages_verdict() {
        let reported = |answer: Value| serde_json::to_string(&answer.to_string()).unwrap();
        assert_eq!(
            page_answer(&reported(json!({ "ok": true, "value": "Settings" }))),
            Ok(json!("Settings"))
        );
        assert_eq!(
            page_answer(&reported(json!({ "ok": false, "error": "2 buttons named Save" }))),
            Err("2 buttons named Save".to_string())
        );
        assert!(page_answer("undefined").is_err());
        assert!(page_answer(&reported(json!({ "value": 1 }))).is_err());
    }

    #[test]
    fn the_script_passes_the_call_as_data_and_survives_a_page_without_the_bridge() {
        let script = page_script(&json!({ "do": "type", "text": "');alert(1);('" }));
        assert!(script.contains(r#"p.run({"do":"type","text":"');alert(1);('"})"#));
        assert!(script.contains("no probe bridge on this page"));
    }

    #[test]
    fn only_the_owner_is_admitted() {
        assert!(admits(501, 501));
        assert!(!admits(0, 501));
        assert!(!admits(502, 501));
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_lives_only_in_a_directory_its_owner_alone_can_enter() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let owner = std::fs::metadata(dir.path()).unwrap().uid();

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(private_dir(dir.path(), owner), Ok(()));

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(dir.path(), owner).unwrap_err().contains("open to other accounts"));

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(private_dir(dir.path(), owner + 1).unwrap_err().contains("another account"));

        let file = dir.path().join("file");
        std::fs::write(&file, "").unwrap();
        assert!(private_dir(&file, owner).unwrap_err().contains("not a directory"));
    }

    /// The promise that matters most: no build anyone ships has the channel.
    ///
    /// Asked of Cargo rather than of the manifest's text, so a feature added
    /// later that pulls `probe` in through another feature is caught too. The
    /// other half of the promise is in `channel.rs`: a build with `probe`
    /// and optimizations on does not compile.
    #[test]
    fn no_feature_but_probe_itself_turns_the_channel_on() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let output = std::process::Command::new(cargo)
            .args([
                "metadata",
                "--no-deps",
                "--offline",
                "--format-version",
                "1",
                "--manifest-path",
            ])
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .output()
            .expect("cargo metadata runs");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
        let features = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == "guac")
            .expect("the guac package")["features"]
            .as_object()
            .unwrap()
            .clone();
        assert!(features.contains_key("probe"), "the probe feature is gone; update this test");

        let enables = |feature: &str| -> Vec<String> {
            let mut seen = vec![feature.to_string()];
            let mut next = 0;
            while next < seen.len() {
                let current = seen[next].clone();
                next += 1;
                for item in features.get(&current).and_then(Value::as_array).into_iter().flatten() {
                    let item = item.as_str().unwrap();
                    if features.contains_key(item) && !seen.iter().any(|s| s == item) {
                        seen.push(item.to_string());
                    }
                }
            }
            seen
        };
        for feature in features.keys().filter(|name| *name != "probe") {
            assert!(
                !enables(feature).iter().any(|name| name == "probe"),
                "the {feature} feature turns the probe on, and with it a control socket"
            );
        }
        assert!(
            features.contains_key("probe-host"),
            "the probe-host feature is gone; update this test"
        );
        for feature in features.keys().filter(|name| *name != "probe-host") {
            assert!(
                !enables(feature).iter().any(|name| name == "probe-host"),
                "the {feature} feature turns probe-host on, and with it a Host that can be told to hurry"
            );
        }
    }
}
