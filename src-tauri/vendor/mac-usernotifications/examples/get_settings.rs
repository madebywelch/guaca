mod common;

use mac_usernotifications::{NotificationSettings, blocking, check_bundle};

fn main() {
    // We can check settings even before requesting auth, but we still need a bundle.
    if let Err(error) = check_bundle() {
        log::error!("Error: {error}");
        return;
    }

    oslog::OsLogger::new("mac-usernotifications")
        .level_filter(log::LevelFilter::Debug)
        .init()
        .unwrap();

    log::info!("get_settings example: starting");

    match blocking::get_notification_settings() {
        Ok(settings) => {
            let NotificationSettings {
                authorization_status,
                alert_enabled,
                badge_enabled,
                sound_enabled,
                lock_screen_enabled,
                notification_center_enabled,
            } = settings;
            log::info!(
                " Notification Settings
                Authorization status : {authorization_status:?}
                Alert (banner)       : {alert_enabled:?}
                Badge                : {badge_enabled:?}
                Sound                : {sound_enabled:?}
                Lock screen          : {lock_screen_enabled:?}
                Notification center  : {notification_center_enabled:?}"
            );
        }
        Err(error) => {
            log::error!("get_notification_settings_blocking failed: {error}");
        }
    }

    log::info!("done");
}
