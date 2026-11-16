use objc2_user_notifications::UNNotificationInterruptionLevel;

/// Controls how a notification interrupts the user.
///
/// Maps to [`UNNotificationInterruptionLevel`](https://developer.apple.com/documentation/usernotifications/unnotificationinterruptionlevel). Available on macOS 12+.
/// On older systems the value is silently ignored by the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InterruptionLevel {
    /// Adds to the notification list without lighting the screen or playing a sound.
    Passive,

    /// Presents immediately, lights the screen, and can play a sound. The default.
    Active,

    /// Presents immediately and bypasses Focus settings.
    TimeSensitive,

    /// Presents immediately, bypasses mute and Do Not Disturb. Requires a special entitlement.
    Critical,
}

impl InterruptionLevel {
    pub(crate) fn to_un_level(self) -> UNNotificationInterruptionLevel {
        match self {
            InterruptionLevel::Passive => UNNotificationInterruptionLevel::Passive,
            InterruptionLevel::Active => UNNotificationInterruptionLevel::Active,
            InterruptionLevel::TimeSensitive => UNNotificationInterruptionLevel::TimeSensitive,
            InterruptionLevel::Critical => UNNotificationInterruptionLevel::Critical,
        }
    }
}
