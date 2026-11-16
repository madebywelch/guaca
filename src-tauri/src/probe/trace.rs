use parking_lot::Mutex;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Clone, Copy, Serialize)]
pub enum Phase {
    #[serde(rename = "trace.opened")]
    TraceOpened,
    #[serde(rename = "notification.response")]
    NotificationResponse,
    #[serde(rename = "app.did_hide")]
    AppDidHide,
    #[serde(rename = "toggle.enter")]
    ToggleEnter,
    #[serde(rename = "toggle.return")]
    ToggleReturn,
    #[serde(rename = "toggle.completed")]
    ToggleCompleted,
    #[serde(rename = "notification.default")]
    NotificationDefault,
    #[serde(rename = "notification.clicked")]
    NotificationClicked,
    #[serde(rename = "window.open")]
    WindowOpen,
    #[serde(rename = "window.reveal")]
    WindowReveal,
}

struct Trace {
    clock: Instant,
    file: File,
    rows: usize,
    broken: bool,
}

const KEPT: usize = 4096;
static TRACE: OnceLock<Option<Mutex<Trace>>> = OnceLock::new();

pub fn opaque(text: &str) -> u64 {
    u64::from(
        text.bytes().fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        }),
    )
}

/// Opened at startup rather than on the first event, so a launch that never
/// traces anything still leaves a file whose marker says the writer worked.
pub fn start() {
    writer();
}

/// What the probe's hello reports, so a missing trace reads as a writer that
/// failed rather than as nothing.
pub fn healthy() -> bool {
    writer().as_ref().is_some_and(|trace| !trace.lock().broken)
}

pub fn event(phase: Phase, id: u64, value: u64) {
    if let Some(trace) = writer() {
        trace.lock().write(phase, id, value);
    }
}

fn writer() -> &'static Option<Mutex<Trace>> {
    TRACE.get_or_init(|| open(Path::new(&std::env::var_os("GUACA_PROBE_TRACE")?)).map(Mutex::new))
}

/// Exclusive, so the trace is never one an earlier launch or anyone else
/// wrote; the probe gives each launch its own path.
fn open(path: &Path) -> Option<Trace> {
    let file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).ok()?;
    let mut trace = Trace { clock: Instant::now(), file, rows: 0, broken: false };
    trace.write(Phase::TraceOpened, 0, 0);
    (!trace.broken).then_some(trace)
}

impl Trace {
    fn write(&mut self, phase: Phase, id: u64, value: u64) {
        if self.rows == KEPT {
            return;
        }
        let main = {
            #[cfg(target_os = "macos")]
            {
                objc2::MainThreadMarker::new().is_some()
            }
            #[cfg(not(target_os = "macos"))]
            {
                false
            }
        };
        let row = serde_json::json!({"phase": phase, "id": id, "value": value, "us": self.clock.elapsed().as_micros() as u64, "main": main});
        if writeln!(self.file, "{row}").and_then(|()| self.file.flush()).is_err() {
            self.broken = true;
        }
        self.rows += 1;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn opaque_ids_match_the_page_without_retaining_the_original() {
        assert_eq!(super::opaque("test"), 2_949_673_445);
    }

    #[test]
    fn a_trace_that_cannot_be_opened_is_reported_as_failed() {
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join("native.jsonl");
        std::fs::write(&taken, "").unwrap();
        assert!(super::open(&taken).is_none());
        assert!(super::open(&dir.path().join("missing/native.jsonl")).is_none());
        assert_eq!(std::fs::read_to_string(&taken).unwrap(), "");
    }

    #[test]
    fn a_new_trace_starts_with_its_opened_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native-2.jsonl");
        let mut trace = super::open(&path).unwrap();
        trace.write(super::Phase::NotificationResponse, 42, 3);
        let rows: Vec<serde_json::Value> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["phase"], "trace.opened");
        assert_eq!(rows[1]["phase"], "notification.response");
        assert_eq!((rows[1]["id"].as_u64(), rows[1]["value"].as_u64()), (Some(42), Some(3)));
        assert!(!trace.broken);
    }
}
