use std::time::Duration;

use mac_usernotifications::{
    Notification, get_delivered_notification_ids, get_pending_notification_ids,
};

mod common;

fn main() {
    if !common::setup(file!()) {
        return;
    }

    // Three notifications sharing a thread -- they should group together.
    for (index, delay_secs) in [(1, 0), (2, 2), (3, 4)] {
        std::thread::sleep(Duration::from_secs(delay_secs));
        let _ = Notification::new()
            .title(format!("Thread demo #{index}"))
            .message("Part of the \"demo.thread\" group.")
            .thread_id("demo.thread")
            .send_blocking();
        log::info!("sent grouped notification {index}/3");
    }

    // Give macOS a moment to record the delivered notifications.
    std::thread::sleep(Duration::from_millis(500));

    let pending = futures_lite::future::block_on(get_pending_notification_ids());
    log::info!("pending notifications ({}): {pending:?}", pending.len());

    let delivered = futures_lite::future::block_on(get_delivered_notification_ids());
    log::info!(
        "delivered notifications ({}): {delivered:?}",
        delivered.len()
    );

    log::info!("Pending  : {pending:?}");
    log::info!("Delivered: {delivered:?}");

    log::info!("done - check Notification Center for the grouped view");
}
