mod common;

use mac_usernotifications::*;

fn main() {
    if !common::setup(file!()) {
        return;
    }

    Notification::default()
        .title("mac usernotifications")
        .subtitle("simple blocking notifications...")
        .message("...almost simple")
        .default_sound()
        .send_blocking()
        .unwrap();
    log::info!("ℹ️ notification 1 sent");

    let response = Notification::default()
        .title("Danger")
        .subtitle("Will Robinson")
        .message("Run away as fast as you can")
        .send_blocking()
        .and_then(|handle| block_on_current(handle.response()))
        .flatten()
        .unwrap();

    if response.is_default_action() {
        log::info!("ℹ️ notification 2 closed via default action");
    }
    if response.is_dismiss_action() {
        log::info!("ℹ️ notification 2 dismissed");
    }
}
