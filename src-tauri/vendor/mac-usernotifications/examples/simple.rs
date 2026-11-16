mod common;

use mac_usernotifications::*;

fn main() {
    if !common::setup(file!()) {
        return;
    }
    block_on_main(async {
        Notification::default()
            .title("mac usernotifications")
            .subtitle("simple async notifications...")
            .message("...almost simple")
            .default_sound()
            .send()
            .await
            .unwrap();
        log::info!("ℹ️ notification sent 1");

        let response = Notification::default()
            .title("Danger")
            .subtitle("Will Robinson")
            .message("Run away as fast as you can")
            .send()
            .await
            .unwrap()
            .response()
            .await
            .unwrap();

        if response.is_default_action() {
            log::info!("ℹ️ notification 2 closed via default action");
        }
        if response.is_dismiss_action() {
            log::info!("ℹ️ notification 2 dismissed");
        }
    });
}
