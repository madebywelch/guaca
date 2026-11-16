mod common;

use std::time::Duration;

use mac_usernotifications::*;

fn main() {
    if !common::setup(file!()) {
        return;
    }
    let response = Notification::new()
        .title("Notification Duration timeout")
        .subtitle("this one should stay for 2s")
        .timeout(Duration::from_secs(2))
        .send_blocking()
        .and_then(|handle| block_on_current(handle.response()))
        .flatten()
        .unwrap();

    if response.is_timed_out() {
        log::info!("timed out as expected");
    } else {
        log::info!("expected timeout, got: {:?}", response.close_reason);
    }

    block_on_main(async {
        let response = Notification::new()
            .title("Notification Duration timeout")
            .subtitle("this one should stay for 2s")
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .unwrap()
            .response()
            .await
            .unwrap();
        if response.is_timed_out() {
            log::info!("timed out as expected");
        } else {
            log::info!("expected timeout, got: {:?}", response.close_reason);
        }
    });
}
