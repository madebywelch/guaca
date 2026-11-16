#![allow(dead_code)]

use std::thread;

use mac_usernotifications::{Notification, blocking, check_bundle};

/// Sets up logging and checks the bundle for the example.
pub fn setup(example_file: &str) -> bool {
    let example_name = std::path::PathBuf::from(example_file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap()
        .to_string();

    // if we don't bundle we must log to stdout
    if let Err(error) = check_bundle() {
        eprintln!(
            "\x1b[1mError:\x1b[0m {error}\n \x1b[1mHelp:\x1b[0m Please run the examples via \x1b[1;35m`./run_bundled_example.sh {example_name}`\x1b[0m"
        );
    }

    oslog::OsLogger::new("mac-usernotifications")
        .level_filter(log::LevelFilter::Debug)
        .init()
        .unwrap();

    log::info!("{example_name} example: starting");

    match blocking::request_auth() {
        Ok(true) => {
            println!("Notification permission granted.");
            true
        }
        Ok(false) => {
            log::warn!(
                "Notification permission denied. \
                 Allow it in System Settings -> Notifications."
            );
            false
        }
        Err(error) => {
            log::error!("Authorization error: {error}");
            false
        }
    }
}

// /// Send `notification` and block until the user responds.
// ///
// /// Equivalent to `block_on_main(notification.send().await?.response().await)`.
// pub fn send_and_await(notification: Notification) -> Result<NotificationResponse, Error> {
//     block_on_main(async { notification.send().await?.response().await })
// }

pub async fn notify_back(title: &str, message: &str) {
    if let Err(error) = Notification::new()
        .title(title)
        .message(message)
        .send()
        .await
    {
        log::error!("notify_back failed: {error}");
    }
}

fn main() {
    unimplemented!("this is just a module");
}

/// Pump the main thread's `NSRunLoop` while `thread` is not finished.
///
/// ## Warning
/// This is a convenience helper for example code only. In proper mac apps you don't need this.
///
/// `NSUserNotificationCenter` delivers delegate callbacks on the main run loop.
/// Call this from the main thread when waiting for a response from a background thread.
pub fn run_main_loop_while<T>(thread: thread::JoinHandle<T>) -> thread::Result<T> {
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
    let run_loop = NSRunLoop::mainRunLoop();
    while !thread.is_finished() {
        let until = NSDate::dateWithTimeIntervalSinceNow(0.05);
        unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &until) };
    }
    thread.join()
}
