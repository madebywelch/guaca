use parking_lot::Mutex;
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Clone, Copy, Serialize)]
pub enum Phase {
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

pub fn event(phase: Phase, id: u64, value: u64) {
    let trace = TRACE.get_or_init(|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(std::env::var_os("GUACA_PROBE_TRACE")?)
            .ok()?;
        Some(Mutex::new(Trace { clock: Instant::now(), file, rows: 0 }))
    });
    let Some(trace) = trace else { return };
    let mut trace = trace.lock();
    if trace.rows == KEPT {
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
    let row = serde_json::json!({"phase": phase, "id": id, "value": value, "us": trace.clock.elapsed().as_micros() as u64, "main": main});
    let _ = writeln!(trace.file, "{row}");
    let _ = trace.file.flush();
    trace.rows += 1;
}

#[cfg(test)]
mod tests {
    #[test]
    fn opaque_ids_match_the_page_without_retaining_the_original() {
        assert_eq!(super::opaque("test"), 2_949_673_445);
    }
}
