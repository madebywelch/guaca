use std::time::Duration;

use mac_usernotifications::{Action, Notification, block_on_current};

mod common;

const ACTION_CLEAR: &str = "action.clear";
const ACTION_KEEP: &str = "action.keep";

fn main() {
    if !common::setup(file!()) {
        return;
    }

    log::info!("sending notification with badge count = 3");

    let _ = Notification::new()
        .title("Badge Demo")
        .message("The app icon badge is now set to 3.")
        .badge(3)
        .default_sound()
        .send_blocking();

    std::thread::sleep(Duration::from_secs(2));

    let response = Notification::new()
        .title("Clear badge?")
        .message("Should the badge be cleared?")
        .action(Action::button(ACTION_CLEAR, "Clear"))
        .action(Action::button(ACTION_KEEP, "Keep"))
        .timeout(Duration::from_secs(60))
        .send_blocking()
        .and_then(|handle| block_on_current(handle.response()))
        .flatten()
        .unwrap();

    if response.action_identifier == ACTION_CLEAR {
        let _ = Notification::new()
            .title("Badge cleared")
            .message("The badge has been reset to zero.")
            .badge(0)
            .send_blocking();
        log::info!("badge cleared");
    } else {
        log::info!("badge left at 3");
    }

    log::info!("done");
}
