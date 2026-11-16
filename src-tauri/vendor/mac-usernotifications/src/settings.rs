//! Types for querying the current notification authorization state.

use objc2_user_notifications::{UNAuthorizationStatus, UNNotificationSetting};

/// The app's authorization to post notifications.
///
/// Returned by [`get_notification_settings`] and [`blocking::get_notification_settings`].
///
/// [`get_notification_settings`]: crate::get_notification_settings
/// [`blocking::get_notification_settings`]: crate::blocking::get_notification_settings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthorizationStatus {
    /// The user has not made a choice yet.
    NotDetermined,
    /// The user denied permission.
    Denied,
    /// The user granted permission.
    Authorized,
    /// Provisional authorization (notifications delivered quietly).
    Provisional,
    /// Ephemeral authorization (granted by App Clips).
    Ephemeral,
    /// An unknown status code was returned by the OS.
    Unknown,
}

impl From<UNAuthorizationStatus> for AuthorizationStatus {
    fn from(status: UNAuthorizationStatus) -> Self {
        match status {
            UNAuthorizationStatus::NotDetermined => AuthorizationStatus::NotDetermined,
            UNAuthorizationStatus::Denied => AuthorizationStatus::Denied,
            UNAuthorizationStatus::Authorized => AuthorizationStatus::Authorized,
            UNAuthorizationStatus::Provisional => AuthorizationStatus::Provisional,
            UNAuthorizationStatus::Ephemeral => AuthorizationStatus::Ephemeral,
            _ => AuthorizationStatus::Unknown,
        }
    }
}

/// Whether a specific notification feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationSettingStatus {
    /// The feature is not supported on this device or OS version.
    NotSupported,
    /// The feature is disabled.
    Disabled,
    /// The feature is enabled.
    Enabled,
    /// An unknown status was returned.
    Unknown,
}

impl From<UNNotificationSetting> for NotificationSettingStatus {
    fn from(setting: UNNotificationSetting) -> Self {
        match setting {
            UNNotificationSetting::NotSupported => NotificationSettingStatus::NotSupported,
            UNNotificationSetting::Disabled => NotificationSettingStatus::Disabled,
            UNNotificationSetting::Enabled => NotificationSettingStatus::Enabled,
            _ => NotificationSettingStatus::Unknown,
        }
    }
}

/// The current notification settings for the app.
///
/// Obtained via [`get_notification_settings`] or [`blocking::get_notification_settings`].
///
/// [`get_notification_settings`]: crate::get_notification_settings
/// [`blocking::get_notification_settings`]: crate::blocking::get_notification_settings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationSettings {
    /// Whether the app is authorized to post notifications.
    pub authorization_status: AuthorizationStatus,
    /// Whether alert notifications (banners) are enabled.
    pub alert_enabled: NotificationSettingStatus,
    /// Whether badge updates are enabled.
    pub badge_enabled: NotificationSettingStatus,
    /// Whether notification sounds are enabled.
    pub sound_enabled: NotificationSettingStatus,
    /// Whether notifications appear on the lock screen.
    pub lock_screen_enabled: NotificationSettingStatus,
    /// Whether notifications appear in Notification Center.
    pub notification_center_enabled: NotificationSettingStatus,
}
