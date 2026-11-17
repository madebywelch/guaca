use std::{
    fmt,
    sync::{Arc, Condvar, Mutex},
};

use crate::NotificationResponse;

type Callback = dyn Fn(&NotificationResponse) + Send + Sync;
static OBSERVER: Mutex<Option<Arc<Observer>>> = Mutex::new(None);

struct Observer {
    callback: Box<Callback>,
    state: Mutex<State>,
    drained: Condvar,
}

struct State {
    active: bool,
    running: usize,
}

pub struct ResponseSubscription(Option<Arc<Observer>>);

impl fmt::Debug for ResponseSubscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseSubscription").finish_non_exhaustive()
    }
}

pub struct ResponseDrain(Arc<Observer>);

impl fmt::Debug for ResponseDrain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResponseDrain").finish_non_exhaustive()
    }
}

pub fn observe_responses(
    callback: impl Fn(&NotificationResponse) + Send + Sync + 'static,
) -> Option<ResponseSubscription> {
    let mut slot = OBSERVER.lock().expect("observer registry poisoned");
    if slot.is_some() {
        return None;
    }
    let observer = Arc::new(Observer {
        callback: Box::new(callback),
        state: Mutex::new(State { active: true, running: 0 }),
        drained: Condvar::new(),
    });
    *slot = Some(observer.clone());
    Some(ResponseSubscription(Some(observer)))
}

impl ResponseSubscription {
    fn deactivate(&mut self) -> Option<Arc<Observer>> {
        let observer = self.0.take()?;
        let mut slot = OBSERVER.lock().expect("observer registry poisoned");
        if slot.as_ref().is_some_and(|current| Arc::ptr_eq(current, &observer)) {
            *slot = None;
        }
        observer.state.lock().expect("observer state poisoned").active = false;
        Some(observer)
    }

    pub fn detach(mut self) -> ResponseDrain {
        ResponseDrain(self.deactivate().expect("subscription already detached"))
    }
}

impl Drop for ResponseSubscription {
    fn drop(&mut self) {
        self.deactivate();
    }
}

impl ResponseDrain {
    pub fn wait(self) {
        let mut state = self.0.state.lock().expect("observer state poisoned");
        while state.running != 0 {
            state = self.0.drained.wait(state).expect("observer state poisoned");
        }
    }
}

struct Invocation(Arc<Observer>);

impl Drop for Invocation {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("observer state poisoned");
        state.running -= 1;
        if state.running == 0 {
            self.0.drained.notify_all();
        }
    }
}

pub(crate) fn dispatch(response: &NotificationResponse) {
    let observer = OBSERVER.lock().expect("observer registry poisoned").clone();
    let Some(observer) = observer else { return };
    {
        let mut state = observer.state.lock().expect("observer state poisoned");
        if !state.active {
            return;
        }
        state.running += 1;
    }
    let invocation = Invocation(observer);
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (invocation.0.callback)(response);
    }))
    .is_err()
    {
        log::error!("native response observer panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegate::route_response;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    fn native_default() {
        route_response(
            "observer-test".into(),
            unsafe { objc2_user_notifications::UNNotificationDefaultActionIdentifier.to_string() },
            None,
        );
    }

    #[test]
    fn native_dispatch_and_scoped_lifetime() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let subscription = observe_responses(move |response| {
            assert!(response.is_default_action());
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        assert!(observe_responses(|_| {}).is_none());
        native_default();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "real native action must reach observer without a response waiter"
        );
        subscription.detach().wait();
        native_default();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let actions = Arc::new(Mutex::new(Vec::new()));
        let recorded = actions.clone();
        let subscription = observe_responses(move |response| {
            recorded
                .lock()
                .unwrap()
                .push((response.is_default_action(), response.is_dismiss_action()));
        })
        .unwrap();
        let (tx, rx) = futures_channel::oneshot::channel();
        crate::delegate::register_response_sender("observer-test".into(), tx);
        native_default();
        assert!(futures_lite::future::block_on(rx).unwrap().is_default_action());
        route_response(
            "observer-test".into(),
            unsafe { objc2_user_notifications::UNNotificationDismissActionIdentifier.to_string() },
            None,
        );
        native_default();
        native_default();
        assert_eq!(
            *actions.lock().unwrap(),
            [(true, false), (false, true), (true, false), (true, false)]
        );
        subscription.detach().wait();

        let owned = Arc::new(Mutex::new(None::<ResponseSubscription>));
        let inside = owned.clone();
        *owned.lock().unwrap() = observe_responses(move |_| {
            drop(inside.lock().unwrap().take());
        });
        native_default();
        assert!(owned.lock().unwrap().is_none());
        let replacement = observe_responses(|_| {}).unwrap();
        drop(replacement);

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let subscription = observe_responses(move |_| {
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
        })
        .unwrap();
        let invocation = std::thread::spawn(native_default);
        entered_rx.recv().unwrap();
        let drain = subscription.detach();
        assert_eq!(drain.0.state.lock().unwrap().running, 1);
        native_default();
        assert!(entered_rx.try_recv().is_err());
        let (drained_tx, drained_rx) = mpsc::channel();
        let stopping = std::thread::spawn(move || {
            drain.wait();
            drained_tx.send(()).unwrap();
        });
        assert!(drained_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        invocation.join().unwrap();
        stopping.join().unwrap();
        drained_rx.recv().unwrap();
        assert!(OBSERVER.lock().unwrap().is_none());

        let subscription = observe_responses(|_| panic!("test observer panic")).unwrap();
        native_default();
        subscription.detach().wait();
        assert!(OBSERVER.lock().unwrap().is_none());
    }
}
