#![warn(missing_docs)]
#![forbid(trivial_numeric_casts, unused_import_braces)]
#![warn(unstable_features)]
#![deny(
    missing_copy_implementations,
    missing_debug_implementations,
    trivial_casts,
    unused_qualifications
)]
#![warn(
    clippy::doc_markdown,
    clippy::semicolon_if_nothing_returned,
    clippy::single_match_else,
    clippy::inconsistent_struct_constructor,
    clippy::map_unwrap_or,
    clippy::match_same_arms
)]

use objc2_foundation::{NSBundle, NSDate, NSDefaultRunLoopMode, NSRunLoop};
use std::future::Future;

mod auth;
mod delegate;
mod error;
mod interrupt;
mod notification;
mod observer;
mod send;
mod settings;
pub mod sound;
mod worker;

pub mod action;
pub mod response;

pub use crate::{
    action::Action,
    auth::{get_notification_settings, request_auth},
    error::Error,
    interrupt::InterruptionLevel,
    notification::Notification,
    observer::{ResponseDrain, ResponseSubscription, observe_responses},
    response::{CloseReason, NotificationResponse},
    send::{
        NotificationHandle, cancel_pending, close_delivered, get_delivered_notification_ids,
        get_pending_notification_ids, send, send_with_actions,
    },
    settings::{AuthorizationStatus, NotificationSettingStatus, NotificationSettings},
};

#[cfg(feature = "blocking-wrappers")]
pub mod blocking {
    pub use crate::{
        auth::{
            get_notification_settings_blocking as get_notification_settings,
            request_auth_blocking as request_auth,
        },
        send::{
            cancel_pending_blocking as cancel_pending, close_delivered_blocking as close_delivered,
            send_blocking as send,
        },
    };
}

pub use futures_lite::future::block_on;

mod runloop_waker {
    use std::task::{RawWaker, RawWakerVTable};

    unsafe fn wake(_: *const ()) {
        if let Some(run_loop) = objc2_core_foundation::CFRunLoop::main() {
            run_loop.wake_up();
        }
    }

    unsafe fn clone(ptr: *const ()) -> RawWaker {
        RawWaker::new(ptr, &VTABLE)
    }

    unsafe fn drop(_: *const ()) {}

    pub static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake, drop);
}

pub fn block_on_main<F: Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, Waker};

    let raw = RawWaker::new(std::ptr::null(), &runloop_waker::VTABLE);
    let waker = unsafe { Waker::from_raw(raw) };
    let mut cx = Context::from_waker(&waker);

    let mut future = std::pin::pin!(future);
    let run_loop = NSRunLoop::mainRunLoop();
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        let until = NSDate::dateWithTimeIntervalSinceNow(1.0);
        unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &until) };
    }
}

pub fn block_on_current<F: Future>(future: F) -> Result<F::Output, Error> {
    if objc2::MainThreadMarker::new().is_some() {
        Ok(block_on_main(future))
    } else if main_run_loop_is_running() {
        Ok(block_on(future))
    } else {
        log::error!("main thread is not pumping");
        Err(Error::MainThreadNotRunning)
    }
}

pub(crate) fn main_run_loop_is_running() -> bool {
    objc2_core_foundation::CFRunLoop::main().is_some_and(|rl| rl.is_waiting())
}

pub fn check_bundle() -> Result<(), Error> {
    NSBundle::mainBundle().bundleIdentifier().ok_or(Error::NoBundleIdentifier)?;
    Ok(())
}
