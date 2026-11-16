use futures_channel::oneshot;

use crate::delegate;
use std::{
    cell::Cell,
    sync::{OnceLock, mpsc},
    thread,
};

type Task = Box<dyn FnOnce() + Send + 'static>;

static WORKER: OnceLock<mpsc::Sender<Task>> = OnceLock::new();

fn handle() -> &'static mpsc::Sender<Task> {
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Task>();
        thread::Builder::new()
            .name("mac-usernotifications".into())
            .spawn(move || {
                log::debug!("thread started");

                delegate::install();
                for task in rx {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(task)) {
                        Ok(()) => log::trace!("task done"),
                        Err(_) => log::error!("task panicked"),
                    }
                }
            })
            .expect("failed to spawn UN worker thread");
        tx
    })
}

pub(crate) fn dispatch<F: FnOnce() + Send + 'static>(task: F) {
    if handle().send(Box::new(task)).is_err() {
        log::error!("dispatch failed (worker thread is gone)");
    }
}

pub(crate) struct Sender<T>(Cell<Option<oneshot::Sender<T>>>);

pub(crate) fn sender<A>() -> (Sender<A>, oneshot::Receiver<A>) {
    let (tx, rx) = oneshot::channel::<A>();
    (Sender(Cell::new(Some(tx))), rx)
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) {
        if let Some(tx) = self.0.take() {
            if tx.send(value).is_err() {
                log::warn!("send failed, sender used twice");
            }
        } else {
            log::warn!("send failed: tx is already taken");
        };
    }
}
