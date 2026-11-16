//! Whether macOS will show a notification Guaca raises, and asking it.
//!
//! The notification plugin answers "granted" on every desktop without asking
//! anyone, and on macOS that was never true: the system keeps a switch per app,
//! and with it off a banner is accepted and then dropped. So a window asks here
//! instead, through the same `UNUserNotificationCenter` the plugin now raises
//! banners with (notify-rust's `preview-macos-un`, turned on in Cargo.toml).
//!
//! Everywhere else there is no per-app answer to read, and every reading is
//! `None`, which tells the window to keep asking the plugin as before.
//!
//! On macOS a banner is also raised here rather than through the plugin. The
//! plugin hands it over with `mac-usernotifications`' blocking send, which
//! refuses ("main thread is not pumping") unless the main run loop happens to
//! be idle at that instant, and drops the error: a delivery the window had
//! already claimed was lost whenever the app was busy, and so was the click on
//! a banner that did appear. The send runs on that crate's own thread and
//! needs nothing from the main one, so here it is awaited, and the window
//! hears whether macOS took it.

use serde::Serialize;

/// The event a click on a banner raised by [`show`] arrives as, carrying the
/// number the window raised it with.
pub const CLICKED: &str = "notification-clicked";

/// What macOS will do with a banner from this app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    /// Banners are on for Guaca.
    Allowed,
    /// Never asked. macOS asks once, and its answer stands until the operator
    /// changes it in System Settings.
    NotAsked,
    /// Anything that ends in no banner: refused, switched off, or kept quietly
    /// in Notification Center. A window that claims a delivery it can only
    /// file there has taken it from a window that could interrupt someone.
    Denied,
    /// Not running from an app bundle, which `UNUserNotificationCenter` needs
    /// and throws without. A build run straight from the checkout is not one.
    Unavailable,
}

/// The current answer, without asking.
pub async fn current() -> Option<Permission> {
    imp::current().await
}

/// The answer, asking first if macOS never has.
pub async fn ask() -> Option<Permission> {
    imp::ask().await
}

/// Opens the pane where the operator turns them back on.
pub async fn open_settings() -> Result<(), String> {
    imp::open_settings().await
}

/// Hands a banner to macOS and returns once macOS has accepted or refused it.
/// With `click`, a click on it is announced as [`CLICKED`] with that number.
/// `None` off macOS, where the window raises it through the plugin as before.
pub async fn show(
    app: tauri::AppHandle,
    title: String,
    body: String,
    click: Option<u32>,
) -> Option<Result<(), String>> {
    #[cfg(feature = "probe")]
    if crate::probe::channel::refused() {
        return Some(Err("refused by the probe".into()));
    }
    imp::show(app, title, body, click).await
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Permission;
    use mac_usernotifications::{
        AuthorizationStatus, NotificationSettingStatus, NotificationSettings,
    };
    use std::path::Path;
    use std::sync::LazyLock;

    /// System Settings, Notifications. The list of apps is one click from it.
    const PANE: &str = "x-apple.systempreferences:com.apple.Notifications-Settings.extension";

    pub async fn current() -> Option<Permission> {
        if !std::env::current_exe().is_ok_and(|exe| inside_app(&exe)) {
            return Some(Permission::Unavailable);
        }
        Some(match mac_usernotifications::get_notification_settings().await {
            Ok(settings) => judge(&settings),
            Err(err) => {
                tracing::warn!(%err, "could not read the notification settings");
                Permission::Unavailable
            }
        })
    }

    pub async fn ask() -> Option<Permission> {
        // One question at a time: two banners raised together would otherwise
        // both find "not asked" and both put the question.
        static ONE: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);
        let _one = ONE.lock().await;
        let now = current().await;
        if now != Some(Permission::NotAsked) {
            return now;
        }
        if let Err(err) = mac_usernotifications::request_auth().await {
            tracing::warn!(%err, "could not ask for permission to notify");
        }
        current().await
    }

    pub async fn show(
        app: tauri::AppHandle,
        title: String,
        body: String,
        click: Option<u32>,
    ) -> Option<Result<(), String>> {
        let sent =
            mac_usernotifications::Notification::new().title(title).message(body).send().await;
        let handle = match sent {
            Ok(handle) => handle,
            Err(err) => {
                tracing::warn!(%err, "macOS did not accept a notification");
                return Some(Err(err.to_string()));
            }
        };
        if let Some(click) = click {
            // Awaited, not blocked on: the response arrives through the
            // delegate on the main run loop, which this only waits to hear.
            tauri::async_runtime::spawn(async move {
                if handle.response().await.is_ok_and(|response| response.is_default_action()) {
                    use tauri::Emitter;
                    if let Err(err) = app.emit(super::CLICKED, click) {
                        tracing::warn!(%err, "could not report a click on a notification");
                    }
                }
            });
        }
        Some(Ok(()))
    }

    pub async fn open_settings() -> Result<(), String> {
        let opened = tokio::process::Command::new("/usr/bin/open")
            .arg(PANE)
            .status()
            .await
            .is_ok_and(|status| status.success());
        if opened {
            Ok(())
        } else {
            Err("Open System Settings, then Notifications, and turn Guaca on.".into())
        }
    }

    /// Whether the executable is `Something.app/Contents/MacOS/binary`.
    fn inside_app(exe: &Path) -> bool {
        exe.parent()
            .filter(|dir| dir.ends_with("Contents/MacOS"))
            .and_then(|dir| dir.parent()?.parent())
            .and_then(Path::extension)
            .is_some_and(|ext| ext == "app")
    }

    /// Allowed only when a banner will actually appear. Provisional delivery
    /// and an alert style of None are both a yes from macOS that files the
    /// notification in Notification Center and interrupts nobody.
    fn judge(settings: &NotificationSettings) -> Permission {
        match settings.authorization_status {
            AuthorizationStatus::NotDetermined => Permission::NotAsked,
            AuthorizationStatus::Authorized | AuthorizationStatus::Ephemeral
                if settings.alert_enabled != NotificationSettingStatus::Disabled =>
            {
                Permission::Allowed
            }
            _ => Permission::Denied,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn settings(
            status: AuthorizationStatus,
            alerts: NotificationSettingStatus,
        ) -> NotificationSettings {
            NotificationSettings {
                authorization_status: status,
                alert_enabled: alerts,
                badge_enabled: NotificationSettingStatus::Enabled,
                sound_enabled: NotificationSettingStatus::Enabled,
                lock_screen_enabled: NotificationSettingStatus::Enabled,
                notification_center_enabled: NotificationSettingStatus::Enabled,
            }
        }

        #[test]
        fn allowed_only_when_a_banner_will_appear() {
            use AuthorizationStatus::*;
            use NotificationSettingStatus::{Disabled, Enabled, NotSupported};
            let cases = [
                (Authorized, Enabled, Permission::Allowed),
                (Ephemeral, Enabled, Permission::Allowed),
                (Authorized, NotSupported, Permission::Allowed),
                (Authorized, Disabled, Permission::Denied),
                (Provisional, Enabled, Permission::Denied),
                (Denied, Enabled, Permission::Denied),
                (Unknown, Enabled, Permission::Denied),
                (NotDetermined, Enabled, Permission::NotAsked),
                (NotDetermined, Disabled, Permission::NotAsked),
            ];
            for (status, alerts, expected) in cases {
                assert_eq!(
                    judge(&settings(status, alerts)),
                    expected,
                    "{status:?} with alerts {alerts:?}"
                );
            }
        }

        #[test]
        fn only_an_app_bundle_can_be_shown_notifications() {
            assert!(inside_app(Path::new("/Applications/Guaca.app/Contents/MacOS/guac")));
            assert!(!inside_app(Path::new("/Users/me/guaca/src-tauri/target/debug/guac")));
            assert!(!inside_app(Path::new("/Applications/Guaca.app/Contents/Resources/guac")));
            assert!(!inside_app(Path::new("/guac")));
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::Permission;

    pub async fn current() -> Option<Permission> {
        None
    }

    pub async fn ask() -> Option<Permission> {
        None
    }

    pub async fn open_settings() -> Result<(), String> {
        Err("Only macOS keeps this switch.".into())
    }

    pub async fn show(
        _app: tauri::AppHandle,
        _title: String,
        _body: String,
        _click: Option<u32>,
    ) -> Option<Result<(), String>> {
        None
    }
}
