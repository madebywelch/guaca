use std::time::Duration;

use mac_usernotifications::Notification;

mod common;

const DEMO_ID: &str = "demo.update";

fn main() {
    if !common::setup(file!()) {
        return;
    }

    let _ = Notification::new()
        .id(DEMO_ID)
        .title("Step 1/3")
        .message("Starting…")
        .send_blocking();
    log::info!("sent step 1");

    std::thread::sleep(Duration::from_secs(2));

    let _ = Notification::new()
        .id(DEMO_ID)
        .title("Step 2/3")
        .message("Still going…")
        .send_blocking();
    log::info!("sent step 2 (replaces step 1)");

    std::thread::sleep(Duration::from_secs(2));

    let _ = Notification::new()
        .id(DEMO_ID)
        .title("Step 3/3 ✓")
        .message("All done.")
        .send_blocking();
    log::info!("sent step 3 (replaces step 2)");
}
