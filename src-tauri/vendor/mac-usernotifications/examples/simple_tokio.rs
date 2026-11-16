use mac_usernotifications::{Notification, block_on_current};

mod common;

fn main() {
    if !common::setup(file!()) {
        return;
    }

    // Multi-thread runtime lives entirely on background threads.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");

    // Spawn the async work onto Tokio, then await its JoinHandle on the main
    // thread while the runLoop is being pumped.
    let handle = rt.spawn(run());
    if let Err(error) = block_on_current(handle) {
        log::error!("tokio task panicked: {error}");
    }
}

async fn run() {
    Notification::default()
        .title("mac usernotifications")
        .subtitle("simple async notifications in tokio...")
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
}
