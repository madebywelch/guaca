use serde::Serialize;

pub const CLICKED: &str = "notification-clicked";

pub fn install(app: &tauri::AppHandle) -> Result<(), String> {
    imp::install(app)
}

pub fn shutdown(app: &tauri::AppHandle) {
    imp::shutdown(app);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    Allowed,
    NotAsked,
    Denied,
    Unavailable,
}

pub async fn current() -> Option<Permission> {
    imp::current().await
}

pub async fn ask() -> Option<Permission> {
    imp::ask().await
}

pub async fn open_settings() -> Result<(), String> {
    imp::open_settings().await
}

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
            #[cfg(feature = "probe")]
            {
                let (click, kind) = response_row(&prefix, response);
                crate::probe::trace::event(
                    crate::probe::trace::Phase::NotificationResponse,
                    click,
                    kind,
                );
            }
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

    fn clicked(
        prefix: &str,
        response: &mac_usernotifications::NotificationResponse,
    ) -> Option<u32> {
        let id = response.notification_id.strip_prefix(prefix)?;
        let click = id.parse::<u32>().ok()?;
        (click.to_string() == id && response.is_default_action()).then_some(click)
    }

    #[cfg(feature = "probe")]
    const RESPONSE_DEFAULT: u64 = 1;
    #[cfg(feature = "probe")]
    const RESPONSE_SESSION: u64 = 2;

    #[cfg(feature = "probe")]
    fn response_row(
        prefix: &str,
        response: &mac_usernotifications::NotificationResponse,
    ) -> (u64, u64) {
        let id = response.notification_id.strip_prefix(prefix);
        let click = id
            .and_then(|id| id.parse::<u32>().ok().filter(|click| click.to_string() == id))
            .map_or(0, u64::from);
        let session = id.is_some();
        let default = if response.is_default_action() { RESPONSE_DEFAULT } else { 0 };
        (click, default | if session { RESPONSE_SESSION } else { 0 })
    }

    fn request_id(actions: Option<&Actions>, click: Option<u32>) -> Option<String> {
        Some(format!("{}{}", actions?.namespace, click?))
    }

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

    fn inside_app(exe: &Path) -> bool {
        exe.parent()
            .filter(|dir| dir.ends_with("Contents/MacOS"))
            .and_then(|dir| dir.parent()?.parent())
            .and_then(Path::extension)
            .is_some_and(|ext| ext == "app")
    }

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

        #[cfg(feature = "probe")]
        #[test]
        fn every_native_response_is_traced_by_kind_session_and_click() {
            use mac_usernotifications::NotificationResponse;
            use objc2_user_notifications::{
                UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier,
            };
            let prefix = "guaca:current-session:";
            let default = unsafe { UNNotificationDefaultActionIdentifier.to_string() };
            let dismiss = unsafe { UNNotificationDismissActionIdentifier.to_string() };
            let row = |id: &str, action: &str| {
                response_row(
                    prefix,
                    &NotificationResponse {
                        notification_id: id.into(),
                        action_identifier: action.into(),
                        reply_text: None,
                        close_reason: None,
                    },
                )
            };
            assert_eq!(
                row("guaca:current-session:42", &default),
                (42, RESPONSE_DEFAULT | RESPONSE_SESSION)
            );
            assert_eq!(row("guaca:current-session:42", &dismiss), (42, RESPONSE_SESSION));
            assert_eq!(
                row("guaca:current-session:042", &default),
                (0, RESPONSE_DEFAULT | RESPONSE_SESSION)
            );
            assert_eq!(row("guaca:old-session:42", &default), (0, RESPONSE_DEFAULT));
            assert_eq!(
                row("5E1A9C2B-0000-4000-8000-000000000000", &default),
                (0, RESPONSE_DEFAULT)
            );
            assert_eq!(row("", "unknown-action"), (0, 0));
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
