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

pub fn install(app: &tauri::AppHandle) -> Result<(), String> {
    imp::install(app)
}

pub fn shutdown(app: &tauri::AppHandle) {
    imp::shutdown(app);
}

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
    use std::sync::{LazyLock, Mutex};
    use tauri::{Emitter, Manager};

    struct Actions {
        namespace: String,
        subscription: Mutex<Option<mac_usernotifications::ResponseSubscription>>,
    }

    impl Actions {
        fn shutdown(&self) {
            let subscription = self.subscription.lock().unwrap().take();
            if let Some(subscription) = subscription {
                subscription.detach().wait();
            }
        }
    }

    pub fn install(app: &tauri::AppHandle) -> Result<(), String> {
        let namespace = format!("guaca:{}:", uuid::Uuid::new_v4());
        let prefix = namespace.clone();
        let emitter = app.clone();
        let subscription = mac_usernotifications::observe_responses(move |response| {
            let Some(click) = clicked(&prefix, response) else { return };
            #[cfg(feature = "probe")]
            crate::probe::trace::event(
                crate::probe::trace::Phase::NotificationDefault,
                u64::from(click),
                0,
            );
            if let Err(err) = emitter.emit(super::CLICKED, click) {
                tracing::warn!(%err, "could not report a click on a notification");
            } else {
                #[cfg(feature = "probe")]
                crate::probe::trace::event(
                    crate::probe::trace::Phase::NotificationClicked,
                    u64::from(click),
                    0,
                );
            }
        })
        .ok_or("A native notification observer is already installed.")?;
        app.manage(Actions { namespace, subscription: Mutex::new(Some(subscription)) });
        Ok(())
    }

    pub fn shutdown(app: &tauri::AppHandle) {
        if let Some(actions) = app.try_state::<Actions>() {
            actions.shutdown();
        }
    }

    /// The click number of a default action on a banner this session raised.
    /// Banners carry no buttons, so macOS never reports their dismissal.
    fn clicked(
        prefix: &str,
        response: &mac_usernotifications::NotificationResponse,
    ) -> Option<u32> {
        let id = response.notification_id.strip_prefix(prefix)?;
        let click = id.parse::<u32>().ok()?;
        (click.to_string() == id && response.is_default_action()).then_some(click)
    }

    /// Without the observer a banner is still shown, only without click-through.
    fn request_id(actions: Option<&Actions>, click: Option<u32>) -> Option<String> {
        Some(format!("{}{}", actions?.namespace, click?))
    }

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
        let mut notification =
            mac_usernotifications::Notification::new().title(title).message(body);
        if let Some(id) = request_id(app.try_state::<Actions>().as_deref(), click) {
            notification = notification.id(&id);
        }
        match mac_usernotifications::send(notification).await {
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(%err, "macOS did not accept a notification");
                return Some(Err(err.to_string()));
            }
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

        #[test]
        fn native_actions_require_this_session_and_a_real_action() {
            use mac_usernotifications::{CloseReason, NotificationResponse};
            use objc2_user_notifications::{
                UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier,
            };
            let prefix = "guaca:current-session:";
            let mut response = NotificationResponse {
                notification_id: format!("{prefix}42"),
                action_identifier: unsafe { UNNotificationDefaultActionIdentifier.to_string() },
                reply_text: None,
                close_reason: None,
            };
            assert_eq!(clicked(prefix, &response), Some(42));
            for id in [
                "guaca:old-session:42",
                "42",
                "guaca:current-session:042",
                "guaca:current-session:-1",
                "guaca:current-session:4294967296",
                "guaca:current-session:42x",
            ] {
                response.notification_id = id.into();
                assert_eq!(clicked(prefix, &response), None);
            }
            response.notification_id = format!("{prefix}42");
            response.action_identifier =
                unsafe { UNNotificationDismissActionIdentifier.to_string() };
            response.close_reason = Some(CloseReason::Dismissed);
            assert_eq!(clicked(prefix, &response), None);
            response.action_identifier.clear();
            assert_eq!(clicked(prefix, &response), None);
            response.close_reason = Some(CloseReason::Expired);
            assert_eq!(clicked(prefix, &response), None);
            response.close_reason = None;
            response.action_identifier = "unknown-action".into();
            assert_eq!(clicked(prefix, &response), None);
        }

        #[test]
        fn a_banner_without_the_observer_is_sent_without_a_click_id() {
            assert_eq!(request_id(None, Some(42)), None);
            let actions = Actions { namespace: "guaca:s:".into(), subscription: Mutex::new(None) };
            assert_eq!(request_id(Some(&actions), Some(42)).as_deref(), Some("guaca:s:42"));
            assert_eq!(request_id(Some(&actions), None), None);
        }

        #[test]
        fn action_shutdown_detaches_once_and_allows_a_new_subscription() {
            let subscription = mac_usernotifications::observe_responses(|_| {}).unwrap();
            let actions = Actions {
                namespace: "test-session".into(),
                subscription: Mutex::new(Some(subscription)),
            };
            assert!(mac_usernotifications::observe_responses(|_| {}).is_none());
            actions.shutdown();
            actions.shutdown();
            assert!(actions.subscription.lock().unwrap().is_none());
            let next = mac_usernotifications::observe_responses(|_| {}).unwrap();
            next.detach().wait();
        }

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

    pub fn install(_app: &tauri::AppHandle) -> Result<(), String> {
        Ok(())
    }

    pub fn shutdown(_app: &tauri::AppHandle) {}

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
