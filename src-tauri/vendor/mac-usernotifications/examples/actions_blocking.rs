use mac_usernotifications::{Action, Notification, block_on_current};

mod common;

const ACTION_OPEN: &str = "action.open";
const ACTION_SAVE: &str = "action.save";
const ACTION_DISCARD: &str = "action.discard";

fn notify_back(title: &str, message: &str) {
    let _ = Notification::new()
        .title(title)
        .message(message)
        .send_blocking();
}

fn main() {
    if !common::setup(file!()) {
        return;
    }

    let notification = Notification::new()
        .title("Download complete")
        .message("report-2026-05.pdf is ready.")
        .action(Action::button(ACTION_OPEN, "Open"))
        .action(Action::button(ACTION_SAVE, "Save to Downloads"))
        .action(Action::button(ACTION_DISCARD, "Discard").requires_authentication())
        .timeout(std::time::Duration::from_secs(60));

    log::info!("sending notification, waiting for user response…");

    let response = notification
        .send_blocking()
        .and_then(|handle| block_on_current(handle.response()))
        .unwrap();

    match response {
        Ok(response) if response.is_default_action() => {
            log::info!("user clicked the notification body");
            notify_back("Opening file…", "report-2026-05.pdf is being opened.");
        }
        Ok(response) if response.is_dismiss_action() => {
            log::info!("user dismissed the notification");
        }
        Ok(response) if response.action_identifier == ACTION_OPEN => {
            log::info!("user chose: open");
            notify_back("Opening file…", "report-2026-05.pdf is being opened.");
        }
        Ok(response) if response.action_identifier == ACTION_SAVE => {
            log::info!("user chose: save");
            notify_back("Saved ✓", "report-2026-05.pdf saved to Downloads.");
        }
        Ok(response) if response.action_identifier == ACTION_DISCARD => {
            log::info!("user chose: discard");
            notify_back("Discarded", "report-2026-05.pdf was deleted.");
        }
        Ok(response) if response.is_timed_out() => {
            log::warn!("timed out waiting for user response");
        }
        Ok(response) => {
            log::warn!("unknown action: {}", response.action_identifier);
        }
        Err(error) => {
            log::error!("error: {error}");
        }
    }

    log::info!("done");
}
