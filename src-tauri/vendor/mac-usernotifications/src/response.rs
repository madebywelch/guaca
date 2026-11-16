//! The response a user gave to a notification.
//!
//! Delivered by the delegate after the user interacts with a notification.
//! Callers receive this via the `Future` returned from `send_with_actions`.

use std::sync::OnceLock;

use objc2_user_notifications::{
    UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier,
};

/// Why a notification was closed without an explicit user action.
///
/// Carried by [`NotificationResponse::close_reason`]. A `None` value means the
/// user took an active action (clicked the body, pressed a button, or submitted
/// a reply).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The notification timed out and was removed by this crate.
    ///
    /// Produced when the duration set via
    /// [`Notification::timeout`](crate::Notification::timeout) elapses.
    Expired,

    /// The user explicitly dismissed the notification (e.g. swiped it away
    /// or pressed "Clear" in Notification Center).
    Dismissed,
}

/// What the user did with a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationResponse {
    /// The unique identifier of the notification this response belongs to.
    ///
    /// This is the same identifier that can be passed to [`close_delivered`] or
    /// [`cancel_pending`]. Useful when the caller did not set an explicit ID via
    /// [`Notification::id`] and therefore did not know the auto-generated UUID.
    ///
    /// [`close_delivered`]: crate::close_delivered
    /// [`cancel_pending`]: crate::cancel_pending
    /// [`Notification::id`]: crate::Notification::id
    pub notification_id: String,

    /// The action identifier the user chose.
    ///
    /// Use [`is_default_action`] and [`is_dismiss_action`] for built-in cases,
    /// or compare directly with custom action identifiers.
    ///
    /// [`is_default_action`]: NotificationResponse::is_default_action
    /// [`is_dismiss_action`]: NotificationResponse::is_dismiss_action
    pub action_identifier: String,

    /// Text entered in a reply action, `None` for button actions, default-action, and dismiss.
    ///
    /// Use [`is_reply`] to check, or call `.reply_text.as_deref()` to borrow as `&str`.
    ///
    /// [`is_reply`]: NotificationResponse::is_reply
    pub reply_text: Option<String>,

    /// Why the notification was closed passively, or `None` if the user took
    /// an explicit action.
    ///
    /// Check [`CloseReason::Expired`] to detect a timeout auto-close, or
    /// [`CloseReason::Dismissed`] for a user-initiated swipe/clear.
    pub close_reason: Option<CloseReason>,
}

/// Returns the dismiss action identifier string, cached for the process lifetime.
fn dismiss_action_id() -> &'static str {
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED.get_or_init(|| {
        // SAFETY: `UNNotificationDismissActionIdentifier` is a valid, non-null
        // `NSString` backed by a well-known Apple framework constant that lives
        // for the lifetime of the process.
        unsafe { UNNotificationDismissActionIdentifier.to_string() }
    })
}

/// Returns the default action identifier string, cached for the process lifetime.
fn default_action_id() -> &'static str {
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED.get_or_init(|| {
        // SAFETY: same rationale as `dismiss_action_id`.
        unsafe { UNNotificationDefaultActionIdentifier.to_string() }
    })
}

impl NotificationResponse {
    /// Build a response from raw macOS delegate data.
    pub(crate) fn from_objc(
        notification_id: String,
        action_id: String,
        reply_text: Option<String>,
    ) -> Self {
        let close_reason = if action_id == dismiss_action_id() {
            Some(CloseReason::Dismissed)
        } else {
            None
        };
        Self {
            notification_id,
            action_identifier: action_id,
            reply_text,
            close_reason,
        }
    }

    /// Construct a synthetic timed-out response for a notification with the given ID.
    ///
    /// Used internally when the caller's timeout elapses and the notification
    /// is removed via [`close_delivered`](crate::close_delivered).
    pub(crate) fn timed_out(notification_id: String) -> Self {
        Self {
            notification_id,
            action_identifier: String::new(),
            reply_text: None,
            close_reason: Some(CloseReason::Expired),
        }
    }

    /// Construct a synthetic dismissed response, used by the poll-based dismiss
    /// fallback for buttonless notifications.
    pub(crate) fn dismissed(notification_id: String) -> Self {
        Self {
            notification_id,
            action_identifier: String::new(),
            reply_text: None,
            close_reason: Some(CloseReason::Dismissed),
        }
    }

    /// Returns `true` if the notification was automatically closed because the
    /// timeout set via [`Notification::timeout`](crate::Notification::timeout) elapsed.
    ///
    /// When this returns `true` the notification banner has already been removed
    /// from the screen by the time the future resolves.
    pub fn is_timed_out(&self) -> bool {
        self.close_reason == Some(CloseReason::Expired)
    }

    /// Returns `true` if the user clicked the notification body (default action).
    ///
    /// Corresponds to [`UNNotificationDefaultActionIdentifier`](https://developer.apple.com/documentation/usernotifications/unnotificationdefaultactionidentifier).
    pub fn is_default_action(&self) -> bool {
        self.action_identifier == default_action_id()
    }

    /// Returns `true` if the user dismissed the notification.
    ///
    /// Corresponds to [`UNNotificationDismissActionIdentifier`](https://developer.apple.com/documentation/usernotifications/unnotificationdismissactionidentifier).
    pub fn is_dismiss_action(&self) -> bool {
        self.close_reason == Some(CloseReason::Dismissed)
    }

    /// Returns `true` if the user submitted text via a reply action.
    pub fn is_reply(&self) -> bool {
        self.reply_text.is_some()
    }
}
