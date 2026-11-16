use crate::{
    Error,
    action::ActionCategory,
    check_bundle, delegate,
    notification::{Notification, NotificationContent},
    response::NotificationResponse,
    worker,
};
use block2::RcBlock;
use futures_channel::oneshot;
use futures_lite::future;
use objc2_foundation::{NSArray, NSError, NSString, NSUUID};
use objc2_user_notifications::{UNNotification, UNNotificationRequest, UNUserNotificationCenter};
use std::{fmt, future::Future, ptr::NonNull, time::Duration};

/// Couples a `request_id` to a response receiver, auto-deregistering on drop.
struct PendingGuard {
    request_id: String,
    rx: oneshot::Receiver<NotificationResponse>,
}

impl PendingGuard {
    fn new(request_id: String, rx: oneshot::Receiver<NotificationResponse>) -> Self {
        Self { request_id, rx }
    }

    /// Consume the guard and return the receiver, skipping deregistration.
    fn into_receiver(self) -> oneshot::Receiver<NotificationResponse> {
        let manual = std::mem::ManuallyDrop::new(self);
        // SAFETY: we are consuming `self` via ManuallyDrop, so Drop won't run.
        // We move `rx` out of the ManuallyDrop'd value before it is forgotten.
        unsafe { std::ptr::read(&manual.rx) }
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        // If the guard is dropped without being consumed (timeout, cancel),
        // clean up the sender so the PENDING map doesn't leak.
        delegate::deregister_response_sender(&self.request_id);
    }
}

/// A notification that has been delivered to the system but not yet responded to.
///
/// Call [`.response().await`](NotificationHandle::response) to wait for the user's interaction,
/// or drop it to stop observing.
/// The notification stays visible but the response sender is cleaned up.
///
pub struct NotificationHandle {
    notification_id: String,
    guard: PendingGuard,
    timeout: Option<Duration>,
    /// `false` when no action buttons were registered, meaning macOS will not fire
    /// `didReceiveNotificationResponse` on a silent dismiss. In that case
    /// [`response`](NotificationHandle::response) falls back to polling
    /// `deliveredNotifications` to detect when the notification disappears.
    has_actions: bool,
}

impl fmt::Debug for NotificationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotificationHandle")
            .field("notification_id", &self.notification_id)
            .field("timeout", &self.timeout)
            .field("has_actions", &self.has_actions)
            .finish()
    }
}

impl NotificationHandle {
    /// The notification's request identifier.
    ///
    /// Can be passed to [`close_delivered`] or [`cancel_pending`] even when no
    /// explicit ID was set via [`Notification::id`].
    pub fn notification_id(&self) -> &str {
        &self.notification_id
    }

    /// Wait for the user's response.
    ///
    /// Resolves when the user interacts with or dismisses the notification.
    ///
    /// If a timeout was set via [`Notification::timeout`], the notification is
    /// automatically removed from the screen when it elapses and this returns
    /// `Ok(response)` where [`NotificationResponse::is_timed_out`] is `true`.
    ///
    /// `UNUserNotificationCenter` has no native TTL; the close is performed
    /// by this crate calling [`close_delivered`] when the timer fires.
    pub async fn response(self) -> Result<NotificationResponse, Error> {
        let notification_id = self.notification_id.clone();
        let receiver = self.guard.into_receiver();
        let delegate_future = async { receiver.await.map_err(|_| Error::NotificationRejected) };

        if let Some(duration) = self.timeout {
            future::or(delegate_future, async move {
                futures_timer::Delay::new(duration).await;
                close_delivered(&notification_id).await;
                Ok(NotificationResponse::timed_out(notification_id))
            })
            .await
        } else if !self.has_actions {
            // No action buttons → macOS never fires didReceiveNotificationResponse on
            // a silent dismiss. Race the delegate future against a poll loop that
            // resolves as soon as the notification disappears from deliveredNotifications.
            future::or(delegate_future, poll_until_dismissed(notification_id)).await
        } else {
            delegate_future.await
        }
    }
}

const DISMISS_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Resolves with a synthetic `Dismissed` response once `notification_id` is no longer
/// present in `deliveredNotifications`.
///
/// Used as a fallback for buttonless notifications where macOS does not fire
/// `didReceiveNotificationResponse` on a silent dismiss.
async fn poll_until_dismissed(notification_id: String) -> Result<NotificationResponse, Error> {
    loop {
        futures_timer::Delay::new(DISMISS_POLL_INTERVAL).await;
        let delivered = get_delivered_notification_ids().await;
        if !delivered.contains(&notification_id) {
            return Ok(NotificationResponse::dismissed(notification_id));
        }
    }
}

/// Core sending logic: dispatch to worker, register response sender, schedule request.
fn send_inner(
    notification: Notification,
    response_tx: Option<oneshot::Sender<NotificationResponse>>,
    request_id: String,
) -> impl Future<Output = Result<(), Error>> + Send + 'static {
    let (scheduled_tx, scheduled_rx) = worker::sender::<Result<(), Error>>();

    worker::dispatch(move || {
        let NotificationContent {
            content,
            actions,
            trigger,
        } = notification.build_content();
        log::debug!("request_id={request_id:?}");

        // Only register a category (and thus CustomDismissAction) when the notification
        // has explicit action buttons. Registering CustomDismissAction on a buttonless
        // notification tells macOS to relaunch the .app bundle on dismiss — causing an
        // infinite restart loop for fire-and-forget notifications.
        if !actions.is_empty() {
            let mut ids: Vec<&str> = actions.iter().map(|act| act.identifier()).collect();
            ids.sort_unstable();
            let category_id = ids.join(",");
            log::debug!("registering synthesised category {category_id:?}");
            ActionCategory::from_actions(&category_id, actions).register_now();
            content.setCategoryIdentifier(&NSString::from_str(&category_id));
        }

        if let Some(tx) = response_tx {
            delegate::register_response_sender(request_id.clone(), tx);
        }

        use objc2_user_notifications::UNTimeIntervalNotificationTrigger;
        let trigger_obj = trigger.map(|delay| {
            UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(
                delay.as_secs_f64().max(0.1),
                false,
            )
        });

        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&request_id),
            &content,
            trigger_obj.as_deref().map(|val| &**val),
        );

        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(
                &request,
                Some(&RcBlock::new(move |err: *mut NSError| {
                    log::trace!("send completed (err.is_null={})", err.is_null());
                    if let Some(err) = NonNull::new(err).map(|ptr| unsafe { ptr.as_ref() }) {
                        let desc = err.localizedDescription();
                        log::error!("notification request rejected: {desc}");
                    }
                    let result = if err.is_null() {
                        Ok(())
                    } else {
                        Err(Error::NotificationRejected)
                    };
                    scheduled_tx.send(result);
                })),
            );
    });

    async move { scheduled_rx.await.map_err(Into::into).flatten() }
}

/// Remove an already-delivered notification from Notification Center by its identifier.
///
/// Fire-and-forget: dispatched to the worker thread, returns immediately.
/// Has no effect if the identifier is unknown.
#[cfg(feature = "blocking-wrappers")]
pub fn close_delivered_blocking(notification_id: &str) {
    let id = notification_id.to_owned();
    worker::dispatch(move || {
        let ids = NSArray::from_retained_slice(&[NSString::from_str(&id)]);
        UNUserNotificationCenter::currentNotificationCenter()
            .removeDeliveredNotificationsWithIdentifiers(&ids);
        log::debug!("removed delivered notification {id:?}");
    });
}

/// Cancel a pending (not yet delivered) notification by its identifier.
///
/// Fire-and-forget: dispatched to the worker thread, returns immediately.
/// Has no effect if the identifier is unknown or already delivered.
#[cfg(feature = "blocking-wrappers")]
pub fn cancel_pending_blocking(notification_id: &str) {
    let id = notification_id.to_owned();
    worker::dispatch(move || {
        let ids = NSArray::from_retained_slice(&[NSString::from_str(&id)]);
        UNUserNotificationCenter::currentNotificationCenter()
            .removePendingNotificationRequestsWithIdentifiers(&ids);
        log::debug!("cancelled pending notification {id:?}");
    });
}

/// Remove a delivered notification.
///
/// Returns a `Future` that resolves once the worker thread has executed the removal.
///
/// Unlike [`close_delivered`], this waits for the worker thread to execute the removal
/// before resolving. Useful when you need to confirm the call was processed.
pub fn close_delivered(notification_id: &str) -> impl Future<Output = ()> + Send + 'static {
    let id = notification_id.to_owned();
    let (tx, rx) = worker::sender::<()>();
    worker::dispatch(move || {
        let ids = NSArray::from_retained_slice(&[NSString::from_str(&id)]);
        UNUserNotificationCenter::currentNotificationCenter()
            .removeDeliveredNotificationsWithIdentifiers(&ids);
        log::debug!("removed delivered notification {id:?}");
        tx.send(());
    });

    async move { rx.await.unwrap_or(()) }
}

/// Cancel a pending notification.
///
/// Returns a `Future` that resolves once the worker thread has executed the cancellation.                 ...
///
/// Unlike [`cancel_pending`], this waits for the worker thread to execute the cancellation before resolving.
pub fn cancel_pending(notification_id: &str) -> impl Future<Output = ()> + Send + 'static {
    let id = notification_id.to_owned();
    let (tx, rx) = worker::sender::<()>();
    worker::dispatch(move || {
        let ids = NSArray::from_retained_slice(&[NSString::from_str(&id)]);
        UNUserNotificationCenter::currentNotificationCenter()
            .removePendingNotificationRequestsWithIdentifiers(&ids);
        log::debug!("cancelled pending notification {id:?}");
        tx.send(());
    });
    async move { rx.await.unwrap_or(()) }
}

/// Return the identifiers of all pending (scheduled but not yet delivered) notifications.
pub fn get_pending_notification_ids() -> impl Future<Output = Vec<String>> + Send + 'static {
    let (tx, rx) = worker::sender::<Vec<String>>();
    worker::dispatch(move || {
        UNUserNotificationCenter::currentNotificationCenter()
            .getPendingNotificationRequestsWithCompletionHandler(&RcBlock::new(
                move |requests: NonNull<NSArray<UNNotificationRequest>>| {
                    let ids: Vec<String> = unsafe { requests.as_ref() }
                        .iter()
                        .map(|req| req.identifier().to_string())
                        .collect();
                    tx.send(ids);
                },
            ));
    });
    async move { rx.await.unwrap_or_default() }
}

/// Return the identifiers of all delivered notifications currently visible in Notification Center.
pub fn get_delivered_notification_ids() -> impl Future<Output = Vec<String>> + Send + 'static {
    let (tx, rx) = worker::sender::<Vec<String>>();
    worker::dispatch(move || {
        UNUserNotificationCenter::currentNotificationCenter()
            .getDeliveredNotificationsWithCompletionHandler(&RcBlock::new(
                move |notifications: NonNull<NSArray<UNNotification>>| {
                    let ids: Vec<String> = unsafe { notifications.as_ref() }
                        .iter()
                        .map(|notif| notif.request().identifier().to_string())
                        .collect();
                    tx.send(ids);
                },
            ));
    });
    async move { rx.await.unwrap_or_default() }
}

/// Schedule a notification for immediate delivery.
///
/// Returns a `Future` that resolves with the notification's request ID once the
/// request is accepted by macOS. The ID can be used with [`close_delivered`] or
/// [`cancel_pending`] even when no explicit ID was set via [`Notification::id`].
///
/// Prefer [`Notification::send`] for the high-level two-phase API.
pub async fn send(notification: Notification) -> Result<String, Error> {
    check_bundle()?;
    let request_id = notification
        .notification_id
        .clone()
        .unwrap_or_else(|| NSUUID::new().UUIDString().to_string());
    let id_copy = request_id.clone();
    send_inner(notification, None, request_id)
        .await
        .map(|()| id_copy)
}

/// Schedule a notification for immediate delivery, blocking the calling thread.
///
/// Returns the notification's request ID on success. See [`send`] for details.
///
/// Prefer [`Notification::send_blocking`] for the high-level two-phase API.
#[cfg(feature = "blocking-wrappers")]
pub fn send_blocking(notification: Notification) -> Result<String, Error> {
    check_bundle()?;
    let request_id = notification
        .notification_id
        .clone()
        .unwrap_or_else(|| NSUUID::new().UUIDString().to_string());
    let id_copy = request_id.clone();
    future::block_on(send_inner(notification, None, request_id)).map(|()| id_copy)
}

/// Schedule a notification and return a [`NotificationHandle`] once it is delivered.
///
/// This is the two-phase entry point: the returned future resolves as soon as macOS
/// accepts the notification request. Call [`.response().await`](NotificationHandle::response)
/// on the handle to wait for the user's interaction, or drop the handle to ignore it.
pub async fn send_and_wait_for_delivery(
    notification: Notification,
) -> Result<NotificationHandle, Error> {
    check_bundle()?;
    let request_id = notification
        .notification_id
        .clone()
        .unwrap_or_else(|| NSUUID::new().UUIDString().to_string());

    let (response_tx, response_rx) = oneshot::channel();
    let timeout = notification.action_timeout;
    let has_actions = !notification.actions.is_empty();
    let guard = PendingGuard::new(request_id.clone(), response_rx);

    send_inner(notification, Some(response_tx), request_id.clone()).await?;

    Ok(NotificationHandle {
        notification_id: request_id,
        guard,
        timeout,
        has_actions,
    })
}

/// Schedule an actionable notification and wait for the user's response.
///
/// This collapses both phases (delivery + response) into one call. For the split
/// two-phase API, use [`Notification::send`] followed by
/// [`NotificationHandle::response`] instead.
///
/// `UNUserNotificationCenter` always delivers callbacks on the main thread's run loop.
/// The main thread must pump `NSRunLoop` while awaiting this future. GUI apps do this
/// automatically; CLI/Tokio apps should use [`crate::block_on_main`] or [`crate::run_main_loop_while`].
///
/// Timeout is set via [`Notification::timeout`]; `None` means wait indefinitely.
/// If a timeout was set, the notification is auto-closed when it elapses and
/// the response has [`NotificationResponse::is_timed_out`] set to `true`.
pub async fn send_with_actions(notification: Notification) -> Result<NotificationResponse, Error> {
    check_bundle()?;

    let request_id = notification
        .notification_id
        .clone()
        .unwrap_or_else(|| NSUUID::new().UUIDString().to_string());
    let (response_tx, response_rx) = oneshot::channel();
    let timeout = notification.action_timeout;
    let has_actions = !notification.actions.is_empty();
    let guard = PendingGuard::new(request_id.clone(), response_rx);

    send_inner(notification, Some(response_tx), request_id.clone()).await?;

    NotificationHandle {
        notification_id: request_id,
        guard,
        timeout,
        has_actions,
    }
    .response()
    .await
}
