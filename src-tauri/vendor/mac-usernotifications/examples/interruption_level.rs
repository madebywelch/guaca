use std::time::Duration;

use mac_usernotifications::{InterruptionLevel, Notification};

mod common;

fn main() {
    if !common::setup(file!()) {
        return;
    }

    log::info!("sending passive notification (no screen wake, no sound)");
    let _ = Notification::new()
        .title("Passive")
        .message("This notification was delivered silently.")
        .interruption_level(InterruptionLevel::Passive)
        .send_blocking();

    std::thread::sleep(Duration::from_secs(3));

    log::info!("sending active notification (normal)");
    let _ = Notification::new()
        .title("Active")
        .message("This is a normal notification (the default level).")
        .interruption_level(InterruptionLevel::Active)
        .default_sound()
        .send_blocking();

    std::thread::sleep(Duration::from_secs(3));

    log::info!("sending time-sensitive notification (breaks through Focus)");
    let _ = Notification::new()
        .title("Time Sensitive")
        .message("This notification breaks through Focus mode settings.")
        .interruption_level(InterruptionLevel::TimeSensitive)
        .default_sound()
        .send_blocking();

    log::info!("done - check Notification Center for the three notifications");
}
