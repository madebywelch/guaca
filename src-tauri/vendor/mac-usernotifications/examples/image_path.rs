use mac_usernotifications::Notification;

mod common;

fn main() {
    if !common::setup(file!()) {
        return;
    }

    // The image is copied into macOS's notification data store when the
    // notification is posted. A copy of the original file is kept next to
    // the example so this example can be run repeatedly.
    let image_src = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/ferris-flat-happy.png"
    );

    // macOS will copy the file, so we hand it a copy so the original stays intact.
    let tmp_copy = std::env::temp_dir().join("ferris-flat-happy-notification.png");
    std::fs::copy(image_src, &tmp_copy).expect("failed to copy Ferris image to temp dir");

    log::info!("sending notification with Ferris attachment");

    let _ = Notification::new()
        .title("Hello from Rust!")
        .message("Ferris says hi.")
        .image_path(tmp_copy.to_string_lossy())
        .default_sound()
        .send_blocking();

    log::info!("done");
}
