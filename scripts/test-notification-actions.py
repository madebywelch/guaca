#!/usr/bin/env python3

import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request


ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "src-tauri/vendor/mac-usernotifications"
CHECKSUM = "08adbb1b9e11709f9ce628dd7471208e5dd3ad35a245dbc81e59a36dce3f3780"


def function(text, signature):
    start = text.index(signature)
    end = text.index("{", start) + 1
    depth = 1
    while depth:
        depth += (text[end] == "{") - (text[end] == "}")
        end += 1
    return text[start:end]


def prepare(crate):
    manifest = (crate / "Cargo.toml").read_text()
    manifest = re.sub(r"\n\[dev-dependencies\.[^\]]+\][\s\S]*?(?=\n\[|\Z)", "\n", manifest)
    (crate / "Cargo.toml").write_text(manifest)
    shutil.copyfile(ROOT / "src-tauri/Cargo.lock", crate / "Cargo.lock")


def instrument(crate, amended):
    send_path = crate / "src/send.rs"
    delegate_path = crate / "src/delegate.rs"
    send = send_path.read_text()
    delegate = delegate_path.read_text()
    getter = "pub fn get_delivered_notification_ids() -> impl Future<Output = Vec<String>> + Send + 'static {"
    assert send.count(getter) == 1
    send = send.replace(getter, "#[cfg(not(test))]\n" + getter)
    if amended:
        route = "route_response(request_id, action_id, reply_text);"
    else:
        route = delegate[delegate.index("            if let Some(tx) = pending()"):delegate.index("            completion_handler.call(())")]
    delegate += """
#[cfg(test)]
pub(crate) fn assay_default(request_id: String) {
    let action_id = unsafe { objc2_user_notifications::UNNotificationDefaultActionIdentifier.to_string() };
    let reply_text: Option<String> = None;
    __ROUTE__
}
""".replace("__ROUTE__", route)
    subscribe = """
        let counted = clicks.clone();
        let subscription = crate::observe_responses(move |response| {
            if response.is_default_action() { counted.fetch_add(1, Ordering::SeqCst); }
        }).unwrap();
""" if amended else ""
    detach = "subscription.detach().wait();" if amended else ""
    count_legacy = "let _ = &observed;" if amended else "if response.is_ok_and(|response| response.is_default_action()) { observed.fetch_add(1, Ordering::SeqCst); }"
    send += r'''
#[cfg(test)]
#[allow(missing_docs)]
pub fn get_delivered_notification_ids() -> impl Future<Output = Vec<String>> + Send + 'static {
    assay::delivered()
}

#[cfg(test)]
mod assay {
    use super::*;
    use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicUsize, Ordering}, mpsc};
    static QUERIES: OnceLock<Mutex<Option<mpsc::Sender<oneshot::Sender<Vec<String>>>>>> = OnceLock::new();

    pub(super) fn delivered() -> impl Future<Output = Vec<String>> + Send + 'static {
        let (tx, rx) = oneshot::channel();
        QUERIES.get().unwrap().lock().unwrap().as_ref().unwrap().send(tx).unwrap();
        async move { rx.await.unwrap_or_default() }
    }

    fn order(poll_first: bool) {
        let id = if poll_first { "poll-first" } else { "default-first" };
        let clicks = Arc::new(AtomicUsize::new(0));
        __SUBSCRIBE__
        let (query_tx, query_rx) = mpsc::channel();
        *QUERIES.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(query_tx);
        let notification = Notification::new().title("test").message("test");
        assert!(notification.actions.is_empty());
        assert!(notification.action_timeout.is_none());
        assert!(notification.trigger.is_none());
        let (tx, rx) = oneshot::channel();
        delegate::register_response_sender(id.into(), tx);
        let handle = NotificationHandle {
            notification_id: id.into(),
            guard: PendingGuard::new(id.into(), rx),
            timeout: notification.action_timeout,
            has_actions: !notification.actions.is_empty(),
        };
        let observed = clicks.clone();
        let reader = std::thread::spawn(move || {
            let response = future::block_on(handle.response());
            let was_default = response.as_ref().is_ok_and(|response| response.is_default_action());
            __COUNT_LEGACY__
            was_default
        });
        let query = query_rx.recv_timeout(Duration::from_secs(3)).expect("real 500 ms poll must issue a query");
        if poll_first {
            query.send(Vec::new()).unwrap();
            assert!(!reader.join().unwrap());
            delegate::assay_default(id.into());
        } else {
            delegate::assay_default(id.into());
            assert!(reader.join().unwrap());
            assert!(query.send(Vec::new()).is_err());
        }
        println!("ACTION_ORDER {} native-defaults={}", id, clicks.load(Ordering::SeqCst));
        assert_eq!(clicks.load(Ordering::SeqCst), 1, "default click must survive polling disappearance");
        __DETACH__
        *QUERIES.get().unwrap().lock().unwrap() = None;
    }

    #[test]
    fn poll_before_default() { order(true); }

    #[test]
    fn default_before_poll() { order(false); }
}
'''.replace("__SUBSCRIBE__", subscribe).replace("__COUNT_LEGACY__", count_legacy).replace("__DETACH__", detach)
    send_path.write_text(send)
    delegate_path.write_text(delegate)


def main():
    if sys.platform != "darwin":
        raise SystemExit("Native notification action verification requires macOS")
    with tempfile.TemporaryDirectory(prefix="guaca-notification-actions-") as directory:
        work = Path(directory)
        archives = list((Path.home() / ".cargo/registry/cache").glob("*/mac-usernotifications-0.3.1.crate"))
        archive = archives[0] if archives else work / "upstream.crate"
        if not archives:
            urllib.request.urlretrieve("https://static.crates.io/crates/mac-usernotifications/mac-usernotifications-0.3.1.crate", archive)
        assert hashlib.sha256(archive.read_bytes()).hexdigest() == CHECKSUM
        pristine = work / "pristine"
        pristine.mkdir()
        with tarfile.open(archive) as tar:
            for member in tar.getmembers():
                relative = Path(member.name).relative_to("mac-usernotifications-0.3.1")
                target = pristine / relative
                assert target.resolve().is_relative_to(pristine.resolve())
                if member.isfile():
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with tar.extractfile(member) as source:
                        target.write_bytes(source.read())
        changed = []
        for original in pristine.rglob("*"):
            if original.is_file():
                path = original.relative_to(pristine)
                if original.read_bytes() != (VENDOR / path).read_bytes():
                    changed.append(str(path))
        assert sorted(changed) == ["src/delegate.rs", "src/lib.rs"], changed
        send = (pristine / "src/send.rs").read_text()
        for signature in ["pub async fn response(self)", "async fn poll_until_dismissed(", "fn into_receiver(self)"]:
            assert function(send, signature) == function((VENDOR / "src/send.rs").read_text(), signature)
        print("Verified published archive checksum and all unchanged upstream files", flush=True)
        env = os.environ.copy()
        env.setdefault("CARGO_TARGET_DIR", str(ROOT / "src-tauri/target/notification-actions"))
        host = next(line.removeprefix("host: ") for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
        for name, amended in [("pristine", False), ("amended", True)]:
            crate = pristine if not amended else work / name
            if amended:
                shutil.copytree(VENDOR, crate)
            prepare(crate)
            instrument(crate, amended)
            metadata = subprocess.run(["cargo", "metadata", "--offline", "--filter-platform", host, "--manifest-path", str(crate / "Cargo.toml"), "--format-version", "1"], capture_output=True, text=True, env=env)
            if metadata.returncode:
                raise SystemExit(metadata.stderr)
            main_lock = (ROOT / "src-tauri/Cargo.lock").read_text()
            locked = (crate / "Cargo.lock").read_text()
            for block in locked.split("[[package]]"):
                package = re.search(r'^name = "([^"]+)"\nversion = "([^"]+)"', block.strip())
                if package:
                    assert f'name = "{package[1]}"\nversion = "{package[2]}"' in main_lock
            result = subprocess.run(["cargo", "test", "--offline", "--locked", "--manifest-path", str(crate / "Cargo.toml"), "--lib", "--", "--test-threads=1", "--nocapture"], capture_output=True, text=True, env=env)
            print(f"=== {name} ===\n{result.stdout}{result.stderr}", flush=True)
            if not amended:
                assert result.returncode == 101
                assert "default click must survive polling disappearance" in result.stdout + result.stderr
                assert "ACTION_ORDER default-first native-defaults=1\nok" in result.stdout
                assert "1 failed" in result.stdout
            else:
                assert result.returncode == 0
        print("Pristine behavioral red and amended real-dispatch green verified", flush=True)


if __name__ == "__main__":
    main()
