//! Sound constants for macOS user notifications.
//!
//! Look under `/System/Library/Sounds/` for the full list of available sounds.
use objc2_foundation::NSString;

/// `/System/Library/Sounds/Basso.aiff`
pub static DEFAULT: &str = "";

/// `/System/Library/Sounds/Basso.aiff`
pub static BASSO: &str = "Basso";

/// `/System/Library/Sounds/Blow.aiff`
pub static BLOW: &str = "Blow";

/// `/System/Library/Sounds/Bottle.aiff`
pub static BOTTLE: &str = "Bottle";

/// `/System/Library/Sounds/Frog.aiff`
pub static FROG: &str = "Frog";

/// `/System/Library/Sounds/Funk.aiff`
pub static FUNK: &str = "Funk";

/// `/System/Library/Sounds/Glass.aiff`
pub static GLASS: &str = "Glass";

/// `/System/Library/Sounds/Hero.aiff`
pub static HERO: &str = "Hero";

/// `/System/Library/Sounds/Morse.aiff`
pub static MORSE: &str = "Morse";

/// `/System/Library/Sounds/Ping.aiff`
pub static PING: &str = "Ping";

/// `/System/Library/Sounds/Sosumi.aiff`
pub static SOSUMI: &str = "Sosumi";

/// `/System/Library/Sounds/Submarine.aiff`
pub static SUBMARINE: &str = "Submarine";

/// `/System/Library/Sounds/Purr.aiff`
pub static PURR: &str = "Purr";

/// `/System/Library/Sounds/Pop.aiff`
pub static POP: &str = "Pop";

/// `/System/Library/Sounds/Tink.aiff`
pub static TINK: &str = "Tink";

/// All built-in macOS system sounds. These correspond to `.aiff` files
/// that ship with every macOS install in `/System/Library/Sounds/`.
pub static SYSTEM_SOUNDS: &[&str] = &[
    BASSO, BLOW, BOTTLE, FROG, FUNK, GLASS, HERO, MORSE, PING, SOSUMI, SUBMARINE, PURR, POP, TINK,
];

pub(crate) fn unnotificationsound(
    name: &str,
) -> Option<objc2::rc::Retained<objc2_user_notifications::UNNotificationSound>> {
    Some(objc2_user_notifications::UNNotificationSound::soundNamed(
        &NSString::from_str(name),
    ))
}
