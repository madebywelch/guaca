use crate::{Error, action::Action, check_bundle, interrupt::InterruptionLevel};

use objc2_foundation::NSString;
use objc2_user_notifications::UNMutableNotificationContent;

use std::time::Duration;

/// A notification with title, body, and optional subtitle / sound.
///
/// Construct with [`Notification::new`] and chain setter methods.
///
/// # Example
///
/// ```no_run
/// use mac_usernotifications::Notification;
///
/// let n = Notification::new()
///     .title("Title")
///     .message("Body text")
///     .subtitle("A subtitle")
///     .sound("Submarine");
/// ```
#[derive(Debug, Default)]
pub struct Notification {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) subtitle: Option<String>,
    pub(crate) sound: Option<String>,
    pub(crate) actions: Vec<Action>,

    /// Timeout for user interaction (actionable notifications only).
    /// `None` means wait indefinitely.
    pub(crate) action_timeout: Option<Duration>,

    /// Optional path to an image to attach to the notification.
    pub(crate) image_path: Option<String>,

    /// Optional delay from now before the notification fires.
    pub(crate) trigger: Option<Duration>,

    /// Groups related notifications in Notification Center.
    pub(crate) thread_id: Option<String>,

    /// Optional explicit request identifier for this notification.
    ///
    /// Re-using an existing identifier replaces the displayed notification in-place.
    pub(crate) notification_id: Option<String>,

    /// Optional badge number for the app icon. `0` clears the badge.
    pub(crate) badge: Option<u32>,

    /// Controls foreground/Focus interruption behavior (macOS 12+).
    pub(crate) interruption_level: Option<InterruptionLevel>,

    /// Sorting hint within a notification group, from 0.0 to 1.0.
    pub(crate) relevance_score: Option<f64>,
}

impl Notification {
    /// Create a new `Notification`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the title.
    pub fn title(mut self, title: impl ToString) -> Self {
        self.title = title.to_string();
        self
    }

    /// Set the body text.
    pub fn message(mut self, message: impl ToString) -> Self {
        self.body = message.to_string();
        self
    }

    /// Set a subtitle.
    pub fn subtitle(mut self, subtitle: impl ToString) -> Self {
        self.subtitle = Some(subtitle.to_string());
        self
    }

    /// Set a subtitle if present.
    pub fn maybe_subtitle(mut self, subtitle: Option<impl ToString>) -> Self {
        if let Some(subtitle) = subtitle {
            self.subtitle = Some(subtitle.to_string());
        }
        self
    }

    /// Add an action button (auto-registered with macOS).
    pub fn action(mut self, action: impl Into<Action>) -> Self {
        self.actions.push(action.into());
        self
    }

    /// Automatically close the notification after `duration` if the user has not
    /// interacted with it yet.
    ///
    /// When the timer fires, the notification banner is removed from the screen
    /// and [`NotificationHandle::response`](crate::send::NotificationHandle::response)
    /// resolves with `Ok(response)` where
    /// [`NotificationResponse::is_timed_out`](crate::NotificationResponse::is_timed_out)
    /// returns `true`.
    ///
    /// Without a timeout, "Clear All" in Notification Center causes an indefinite wait.
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.action_timeout = Some(duration);
        self
    }

    /// Play the default system sound.
    pub fn default_sound(mut self) -> Self {
        self.sound = Some("".to_string());
        self
    }

    /// Play a named sound from the app bundle or system library.
    pub fn sound<S: Into<String>>(mut self, sound: S) -> Self {
        self.sound = Some(sound.into());
        self
    }

    /// Play a named sound from the app bundle or system library, if present.
    pub fn maybe_sound<S: Into<String>>(mut self, sound: Option<S>) -> Self {
        if let Some(sound) = sound {
            self.sound = Some(sound.into());
        }
        self
    }

    /// Attach an image to the notification by file path.
    ///
    /// macOS copies the file at `path` into its own notification data store.
    /// The framework manages the stored copy; do not rely on the original path
    /// remaining accessible after the notification is posted.
    pub fn image_path(mut self, path: impl ToString) -> Self {
        self.image_path = Some(path.to_string());
        self
    }

    /// Schedule the notification to fire after the given delay from now.
    pub fn schedule_in(mut self, delay: Duration) -> Self {
        self.trigger = Some(delay);
        self
    }

    /// Group this notification with others sharing the same thread identifier.
    ///
    /// macOS displays grouped notifications collapsed in Notification Center.
    /// Use a stable per-conversation or per-topic string, e.g. `"chat.alice"`.
    pub fn thread_id(mut self, id: impl ToString) -> Self {
        self.thread_id = Some(id.to_string());
        self
    }

    /// Set an explicit notification identifier.
    ///
    /// Posting a new notification with the same identifier replaces any previously
    /// displayed notification with that ID. Useful for updating live statuses
    /// (e.g. a progress indicator or a changing score).
    ///
    /// If not set, a fresh UUID is generated per [`send`](Self::send).
    pub fn id(mut self, id: &str) -> Self {
        self.notification_id = Some(id.to_owned());
        self
    }

    /// Set the app icon badge number. Use `0` to clear the badge.
    pub fn badge(mut self, count: u32) -> Self {
        self.badge = Some(count);
        self
    }

    /// Set the interruption level for this notification (macOS 12+).
    ///
    /// Controls whether the notification breaks through Focus modes.
    /// See [`InterruptionLevel`] for the available levels.
    pub fn interruption_level(mut self, level: InterruptionLevel) -> Self {
        self.interruption_level = Some(level);
        self
    }

    /// Set the relevance score for sorting within a notification group.
    ///
    /// Value between `0.0` and `1.0`. Higher scores appear first when notifications
    /// are grouped by thread identifier.
    pub fn relevance_score(mut self, score: f64) -> Self {
        self.relevance_score = Some(score);
        self
    }

    pub(crate) fn build_content(self) -> NotificationContent {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&self.title));
        content.setBody(&NSString::from_str(&self.body));

        let to_nsstring = |string: String| NSString::from_str(&string);

        if let Some(ref sub) = self.subtitle.map(to_nsstring) {
            content.setSubtitle(sub);
        }

        if let Some(ref id) = self.thread_id.map(to_nsstring) {
            content.setThreadIdentifier(id);
        }

        if let Some(sound) = self.sound {
            content.setSound(crate::sound::unnotificationsound(&sound).as_deref());
        }

        if let Some(ref path) = self.image_path {
            use objc2_foundation::{NSArray, NSURL};
            use objc2_user_notifications::UNNotificationAttachment;
            log::debug!("attaching image: {path:?}");
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));

            match unsafe {
                UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(
                    &NSString::from_str("image"),
                    &url,
                    None,
                )
            } {
                Ok(attachment) => {
                    let attachments = NSArray::from_retained_slice(&[attachment]);
                    content.setAttachments(&attachments);
                }
                Err(err) => {
                    log::error!("failed to create notification attachment for {path:?}: {err:?}");
                }
            }
        }

        if let Some(count) = self.badge {
            content.setBadge(Some(&objc2_foundation::NSNumber::new_u32(count)));
        }

        if let Some(level) = self.interruption_level {
            content.setInterruptionLevel(level.to_un_level());
        }

        if let Some(score) = self.relevance_score {
            content.setRelevanceScore(score);
        }

        NotificationContent {
            content,
            actions: self.actions,
            trigger: self.trigger,
        }
    }
}

/// Sending
impl Notification {
    /// Send the notification and return a [`NotificationHandle`](`crate::NotificationHandle`) once it is delivered.
    ///
    /// The future resolves as soon as macOS accepts the notification request.
    /// Call [`.response().await`](crate::send::NotificationHandle::response) on the handle to wait for the user's interaction,
    /// or drop it to ignore the response.
    ///
    ///
    /// `UNUserNotificationCenter` always delivers callbacks on the main thread's run loop.
    /// The main thread must pump `NSRunLoop` while awaiting the response future.
    /// GUI apps do this automatically; CLI/Tokio apps should use
    /// [`crate::block_on_main`] or [`crate::run_main_loop_while`].
    pub async fn send(self) -> Result<crate::send::NotificationHandle, Error> {
        use crate::send::send_and_wait_for_delivery;
        check_bundle()?;
        send_and_wait_for_delivery(self).await
    }

    /// Send the notification, blocking until delivered, then return a [`NotificationHandle`](`crate::NotificationHandle`).
    ///
    /// Waits for macOS to accept the notification request, then returns a handle. Call
    /// [`crate::block_on_main`]`(handle.response())` to block waiting for the user's
    /// response, or drop the handle to ignore it.
    #[cfg(feature = "blocking-wrappers")]
    pub fn send_blocking(self) -> Result<crate::send::NotificationHandle, Error> {
        use crate::send::send_and_wait_for_delivery;
        check_bundle()?;
        crate::block_on_current(send_and_wait_for_delivery(self))?
    }
}

/// The result of building a [`Notification`] into its Objective-C content representation.
///
/// Returned by [`Notification::build_content`].
pub(crate) struct NotificationContent {
    pub(crate) content: objc2::rc::Retained<UNMutableNotificationContent>,
    pub(crate) actions: Vec<Action>,
    pub(crate) trigger: Option<Duration>,
}

#[cfg(test)]
mod test_notification_builder {
    use crate::sound;

    use super::Notification;

    #[test]
    fn maybe_subtitle_sets_when_some_skips_when_none() {
        let with = Notification::new().maybe_subtitle(Some("Sub"));
        let without = Notification::new().maybe_subtitle(Option::<&str>::None);
        assert_eq!(with.subtitle.as_deref(), Some("Sub"));
        assert!(without.subtitle.is_none());
    }

    #[test]
    fn maybe_sound_sets_when_some_skips_when_none() {
        let with = Notification::new().maybe_sound(Some("Funk"));
        let without = Notification::new().maybe_sound(Option::<&str>::None);
        assert_eq!(with.sound, Some(sound::FUNK.into()));
        assert!(without.sound.is_none());
    }

    #[test]
    fn sound_via_str_resolves_named_variant() {
        // Exercises the From<&str> → Sound conversion through the builder.
        // A naive impl that always produces Custom would pass string equality
        // checks but fail here.
        let notif = Notification::new().sound("Basso");
        assert_eq!(notif.sound, Some(sound::BASSO.into()));
    }
}
