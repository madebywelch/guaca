use serde::{Deserialize, Serialize};
use serde_json::Value;

mod toggle;
#[cfg(feature = "probe")]
pub mod trace;

#[cfg(feature = "probe")]
pub mod channel;

pub const SOCKET_ENV: &str = "GUACA_PROBE_SOCKET";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Envelope {
    pub id: u64,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Request {
    Hello,
    State,
    Page { window: String, call: Value },
    ClickIcon,
    Close { window: String },
    Hide,
    Quit,
    Busy { ms: u64 },
    RefuseNextBanner,
}

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

    pub fn line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("a reply serializes");
        line.push('\n');
        line
    }
}

pub fn parse(line: &str) -> Result<Envelope, Reply> {
    serde_json::from_str(line).map_err(|err| {
        let id = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|value| value.get("id")?.as_u64())
            .unwrap_or(0);
        Reply::to(id, Err(format!("not a probe request: {err}")))
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub identifier: String,
    pub pid: u32,
    pub windows: Vec<Window>,
    pub notifications: Value,
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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Menubar {
    pub glyph: String,
    pub title: Option<String>,
    pub tooltip: String,
    pub attached: bool,
    pub panel: bool,
    pub panel_visible: bool,
}

pub fn page_script(call: &Value) -> String {
    format!(
        "(function(){{var p=window.__guacaProbe;return JSON.stringify(p?p.run({call}):{{ok:false,error:'no probe bridge on this page'}});}})()"
    )
}

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

pub fn admits(peer: u32, owner: u32) -> bool {
    peer == owner
}

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
            (r#"{"id":10,"op":"refuseNextBanner"}"#, Request::RefuseNextBanner),
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
