use std::pin::Pin;

use tokio::sync::futures::Notified;
use tokio::sync::Notify;

pub(super) struct Watch<'a, S> {
    woken: Pin<Box<Notified<'a>>>,
    stop: S,
}

impl<'a, S> Watch<'a, S> {
    pub(super) fn arm(signal: &'a Notify, read: impl FnOnce() -> S) -> Self {
        let mut woken = Box::pin(signal.notified());
        woken.as_mut().enable();
        Self { stop: read(), woken }
    }

    pub(super) fn stop(&self) -> &S {
        &self.stop
    }

    pub(super) async fn woken(self) {
        self.woken.await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use futures_util::FutureExt;

    use super::*;

    #[test]
    fn a_stop_that_lands_right_after_the_read_still_wakes_the_watch() {
        let signal = Notify::new();
        let stopped = AtomicBool::new(false);
        let watch = Watch::arm(&signal, || {
            let seen = stopped.load(Ordering::SeqCst);
            stopped.store(true, Ordering::SeqCst);
            signal.notify_waiters();
            seen
        });
        assert!(!watch.stop(), "the read saw a stop that had not landed yet");
        assert!(
            watch.woken().now_or_never().is_some(),
            "a stop announced after the read was lost, so the wait would never end"
        );
    }
}
