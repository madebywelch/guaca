mod common;

use mac_usernotifications::{Notification, sound};

fn main() {
    if !common::setup(file!()) {
        return;
    }

    // Start with the default system notification sound.
    log::info!("Playing: Default");
    let _ = Notification::new()
        .title("Sound Demo")
        .subtitle("Default")
        .message("The default system notification sound")
        .default_sound()
        .send_blocking();

    std::thread::sleep(std::time::Duration::from_secs(2));

    // Then cycle through every built-in macOS system sound.
    for sound in sound::SYSTEM_SOUNDS {
        // let name = sound.sound_name().unwrap();
        log::info!("Playing: {sound}");

        let _ = Notification::new()
            .title("Sound Demo")
            .subtitle(sound)
            .message(format!("/System/Library/Sounds/{sound}.aiff"))
            .sound(&**sound)
            .send_blocking();
        log::info!("Done Playing: {sound}");

        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    log::info!("Done! Played {} sounds.", sound::SYSTEM_SOUNDS.len() + 1);
}
