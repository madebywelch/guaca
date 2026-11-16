use std::time::Duration;

use futures_lite::future;
use mac_usernotifications::{
    Action, InterruptionLevel::Critical, Notification, block_on_current, cancel_pending,
    close_delivered,
};

mod common;

const ACTION_CLOSE: &str = "action.close";
const ACTION_LEAVE: &str = "action.leave";

fn main() {
    if !common::setup(file!()) {
        return;
    }

    let id = "demo.close";

    // Post a persistent notification.
    let _ = Notification::new()
        .id(id)
        .title("I am still here")
        .message("Waiting to be closed programmatically.")
        .interruption_level(Critical)
        .send_blocking();

    log::info!("persistent notification delivered with id={id:?}");

    std::thread::sleep(Duration::from_secs(2));

    // Demo: schedule a future notification, then cancel it before it fires.
    let pending_id = Notification::new()
        .title("You should never see this")
        .message("This scheduled notification will be cancelled immediately.")
        .schedule_in(Duration::from_secs(60))
        .send_blocking()
        .unwrap()
        .notification_id()
        .to_owned();

    log::info!("scheduled pending notification id={pending_id:?}, cancelling now...");
    future::block_on(cancel_pending(&pending_id));
    log::info!("cancel_pending_async resolved - pending notification removed");

    // Ask the user what to do with the first notification.
    let response = Notification::new()
        .title("Close example")
        .message("Should the first notification be removed?")
        .action(Action::button(ACTION_CLOSE, "Close it"))
        .action(Action::button(ACTION_LEAVE, "Leave it"))
        .interruption_level(Critical)
        .timeout(Duration::from_secs(60))
        .send_blocking()
        .and_then(|handle| block_on_current(handle.response()))
        .flatten()
        .unwrap();

    if response.action_identifier == ACTION_CLOSE {
        // Use the async variant to confirm removal was dispatched.
        future::block_on(close_delivered(id));
        log::info!("close_delivered_async resolved for {id:?}");
        let _ = Notification::new()
            .title("Closed")
            .message("The first notification has been removed.")
            .send_blocking();
    } else {
        log::info!("user chose to leave the notification");
        let _ = Notification::new()
            .title("Left alone")
            .message("The first notification was left in Notification Center.")
            .send_blocking();
    }

    log::info!("done");
}
