//! A Rust wrapper around [`UNUserNotificationCenter`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter),
//! designed for use in [notify-rust](https://docs.rs/notify-rust).
//!
//! # Bundling Requirement
//!
//! Contrary to [mac-notification-sys](https://docs.rs/mac-notification-sys),
//! this crate requires that the binary is bundled  and be code-signed,
//! an ad-hoc signature is sufficient.
//! See the bundled examples for how to set this up with `cargo-bundle`.
//!
//! # Quick start
//!
//! ```no_run
//! # use mac_usernotifications::{Action, blocking, block_on_main, Notification, check_bundle};
//! # use std::time::Duration;
//! # fn main() {
//! // 1. verify the process has a bundle identifier
//! check_bundle().unwrap();
//!
//! // 2. verify user gave permission
//! blocking::request_auth().unwrap();
//!
//! // 3a. fire-and-forgeta (handle.notification_id() has the UUID for later use)
//! let handle = Notification::new()
//!     .title("Hello")
//!     .message("World")
//!     .send_blocking()
//!     .unwrap();
//!
//! println!("notification id: {}", handle.notification_id());
//!
//! // 3b. actionable: use block_on_main to drive the run loop while waiting for the response
//! let response = block_on_main(async {
//!     Notification::new()
//!         .title("Pick one")
//!         .action(Action::button("ok", "OK"))
//!         .action(Action::button("cancel", "Cancel"))
//!         .timeout(Duration::from_secs(30)) // 4. always set a timeout for actionable notifications
//!         .send()
//!         .await?
//!         .response()
//!         .await
//! }).unwrap();
//!
//! println!("User chose: {}", response.action_identifier);
//! # }
//! ```
//!
//! # Threading model
//!
//! macOS delivers [`didReceiveNotificationResponse`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenterdelegate/usernotificationcenter(_:didreceive:withcompletionhandler:)) on the main thread's [`NSRunLoop`](https://developer.apple.com/documentation/foundation/nsrunloop),
//! regardless of which thread the delegate was registered from ([Apple docs](https://developer.apple.com/documentation/usernotifications/unusernotificationcenterdelegate)).
//! The main thread's run loop must be spinning whenever you expect the user to interact with a notification.
//!
//! This crate uses a lazily-created worker thread for all Objective-C calls, to enable async APIs.
//!
//! ## GUI apps (`AppKit` / `SwiftUI` / `Tauri`)
//!
//! The framework drives the main run loop automatically. `send`, `send_blocking`, and
//! `response().await` all work from any thread without extra setup.
//!
//! ## Headless / background-only apps
//!
//! No framework drives the run loop, so you must do it yourself. Use [`block_on_main`]
//! to run an async expression on the main thread while pumping the run loop:
//!
//! ```no_run
//! # use mac_usernotifications::{Notification, block_on_main};
//! # fn main() {
//! let response = block_on_main(async {
//!     Notification::new()
//!         .title("Hello")
//!         .send().await?
//!         .response().await
//! });
//! # }
//! ```
//!
//! Or for Tokio, keep the main thread free for [`run_main_loop_while`] and run the
//! runtime entirely on background threads. See `examples/simple_tokio.rs`.
//!
//! ## Known pitfalls
//!
//! | Scenario | Symptom | Fix |
//! |---|---|---|
//! | `response().await` called with nothing pumping the main run loop | future never resolves | Use [`block_on_main`] or [`run_main_loop_while`] on the main thread |
//! | `#[tokio::main]` — main thread is inside Tokio | callbacks never fire | Use `new_multi_thread()` and keep main free for [`run_main_loop_while`] |
//! | User clicks **"Clear All"** in Notification Center | future never resolves | Always set a timeout via [`Notification::timeout`] for actionable notifications |

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
    //! Blocking wrappers for fire-and-forget operations.
    //!
    //! These are safe to call from any thread. None of them wait for a
    //! notification response — use [`crate::block_on_main`] with the async
    //! API for that.
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

/// Set the application which delivers or schedules a notification
/// A [`RawWakerVTable`](std::task::RawWakerVTable) whose `wake` calls [`CFRunLoop::wake_up`](objc2_core_foundation::CFRunLoop::wake_up) on the main run loop.
///
/// The data pointer is always null; the main run loop is a global.
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

/// Run a future to completion on the main thread while pumping [`NSRunLoop`](https://developer.apple.com/documentation/foundation/nsrunloop).
///
/// ## Thread Safety
/// **Must be called from the main thread.**
/// Uses a waker that calls [`CFRunLoop::wake_up`](objc2_core_foundation::CFRunLoop::wake_up) on the main run loop,
/// so the run loop sleep is interrupted as soon as the future signals readiness, no busy-polling.
/// GUI apps (`Tauri`, `AppKit`, `SwiftUI`) pump [`RunLoop`](https://developer.apple.com/documentation/foundation/runloop) automatically.
///
/// **Use [`dynamic_block_on`] instead if the main thread may not be running.**
pub fn block_on_main<F: Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, Waker};

    let raw = RawWaker::new(std::ptr::null(), &runloop_waker::VTABLE);
    // SAFETY: the vtable functions are correct and the null data pointer is intentional
    // (wake_up targets the global main run loop, no per-waker state needed).
    let waker = unsafe { Waker::from_raw(raw) };
    let mut cx = Context::from_waker(&waker);

    let mut future = std::pin::pin!(future);
    let run_loop = NSRunLoop::mainRunLoop();
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        // Sleep up to 1 s; the waker interrupts this early whenever the future is ready.
        let until = NSDate::dateWithTimeIntervalSinceNow(1.0);
        unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &until) };
    }
}

/// Block on the future, using [`block_on_main`] if the current thread is the main thread.
///
/// Returns `None` if the main thread is not pumping.
pub fn block_on_current<F: Future>(future: F) -> Result<F::Output, Error> {
    if objc2::MainThreadMarker::new().is_some() {
        // we are on the main thread, so we can safely block on the future
        Ok(block_on_main(future))
    } else if main_run_loop_is_running() {
        // we can safely block on the future here, as we know the main thread is pumping
        Ok(block_on(future))
    } else {
        log::error!("main thread is not pumping");
        Err(Error::MainThreadNotRunning)
    }
}

/// Returns `true` if the main thread's run loop is active and able to deliver callbacks.
///
/// macOS always delivers notification responses on the main thread's run loop.
/// If nothing is running it (no AppKit/SwiftUI framework, no explicit [`run_main_loop_while`] call),
/// response futures will never resolve.
pub(crate) fn main_run_loop_is_running() -> bool {
    objc2_core_foundation::CFRunLoop::main().is_some_and(|rl| rl.is_waiting())
}

/// Verify the process has a bundle identifier.
///
/// [`UNUserNotificationCenter`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter) requires this and crashes without it.
pub fn check_bundle() -> Result<(), Error> {
    NSBundle::mainBundle()
        .bundleIdentifier()
        .ok_or(Error::NoBundleIdentifier)?;
    Ok(())
}
