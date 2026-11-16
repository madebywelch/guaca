<div align="center">

# mac-usernotifications

[![Build](https://github.com/hoodie/mac-usernotifications/actions/workflows/build.yml/badge.svg)](https://github.com/hoodie/mac-usernotifications/actions/workflows/build.yml)
[![Semver Checks](https://github.com/hoodie/mac-usernotifications/actions/workflows/semver-checks.yml/badge.svg)](https://github.com/hoodie/mac-usernotifications/actions/workflows/semver-checks.yml)
![maintenance](https://img.shields.io/maintenance/yes/2027)

[![Crates.io](https://img.shields.io/crates/d/mac-usernotifications)](https://crates.io/crates/mac-usernotifications)
[![version](https://img.shields.io/crates/v/mac-usernotifications)](https://crates.io/crates/mac-usernotifications/)
[![documentation](https://img.shields.io/badge/docs-latest-blue.svg)](https://docs.rs/mac-usernotifications/)

</div>

A wrapper around macOS [`UNUserNotificationCenter`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter) in Rust.

- actionable notifications (buttons, reply fields)
- async and blocking
- updating of notifications
- MacOS only (iOS only features are intentionally omitted)

## Usage

> [!Note]
> Notifications require a bundled app.
> Use `./run_bundled_example.sh $EXAMPLE_NAME` to bundle and run the examples.

### Simple async notifications and closing handling

```rs
Notification::default()
    .title("mac usernotifications")
    .subtitle("simple async notifications...")
    .message("...almost simple")
    .default_sound()
    .send().await?;

log::info!("ℹ️ notification sent 1");

let response = Notification::default()
    .title("Danger")
    .subtitle("Will Robinson")
    .message("Run away as fast as you can")
    .send().await?
    .response().await?;

if response.is_default_action() {
    log::info!("ℹ️ notification 2 closed via default action");
}

if response.is_dismiss_action() {
    log::info!("ℹ️ notification 2 dismissed");
}
```

### Action buttons (blocking)

```rs
let notification = Notification::new()
    .title("Download complete")
    .message("report-2026-05.pdf is ready.")
    .action(Action::button(ACTION_OPEN, "Open"))
    .action(Action::button(ACTION_SAVE, "Save to Downloads"))
    .action(Action::button(ACTION_DISCARD, "Discard").requires_authentication())
    .timeout(std::time::Duration::from_secs(60));

let response = notification
    .send_blocking()
    .and_then(|sent| sent.response_blocking());

match response {
    Ok(response) if response.action_identifier == ACTION_OPEN => { /* … */ }
    Ok(response) if response.action_identifier == ACTION_SAVE => { /* … */ }
    Ok(response) if response.action_identifier == ACTION_DISCARD => { /* … */ }
    Err(mac_usernotifications::Error::ResponseTimeout) => { /* timed out */ }
    _ => {}
}
```

### Reply actions (async)

Reply actions open an inline text field; the typed text arrives in `response.reply_text`.

```rs
Notification::new()
    .title("New message from Hendrik")
    .message("Hey, are you coming tonight?")
    .action(Action::reply(ACTION_REPLY, "📝 Reply", "Send", "Type a reply…"))
    .action(Action::button(ACTION_LIKE, "👍 Like"))
    .action(Action::button(ACTION_TRASH, "🗑️ Delete").requires_authentication())
    .timeout(Duration::from_secs(60))
    .send()
    .await
```

## Requirements

- A valid `.app` bundle with a [`CFBundleIdentifier`](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleidentifier) ([`UNUserNotificationCenter`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter) requires this) (use [`cargo-bundle`](https://github.com/burtonageo/cargo-bundle) to set it up)
- Notification permission granted by the user

## Threading

Callbacks always arrive on the main thread's run loop. GUI apps (AppKit, SwiftUI, Tauri) handle this automatically. CLI tools and Tokio apps need extra care; see the [crate docs](https://docs.rs/mac-usernotifications) for details.

> [!Warning]
> `#[tokio::main]` occupies the main thread, so callbacks never fire. Use a `new_multi_thread` runtime on background threads and keep the main thread free for `NSRunLoop`. See `examples/simple_tokio.rs`.

## Related Work

- **[`mac-notification-sys`](https://crates.io/crates/mac-notification-sys)** \
  a thin Objective-C wrapper around [`NSUserNotificationCenter`](https://developer.apple.com/documentation/foundation/nsusernotificationcenter); used as the macOS backend for
  `notify-rust`. Supports basic fire-and-forget delivery with an optional sound; response handling is
  limited to a `NotificationResponse` enum (clicked, button clicked, replied, etc.) with no async
  support or per-notification typed responses.

- **[`user-notify`](https://crates.io/crates/user-notify)** \
  most similar to this crate,
  wraps [`UNUserNotificationCenter`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter) and provides async response handling using Tokio.

- **[`mac-usernotifications`](https://crates.io/crates/mac-usernotifications)** _THIS CRATE_ \
  wraps [`UNUserNotificationCenter`](https://developer.apple.com/documentation/usernotifications/unusernotificationcenter) with support for fire-and-forget and actionable
  notifications (buttons, reply fields), both async and blocking, with per-notification typed
  responses and optional timeouts. Does not require Tokio; async support is runtime-agnostic and
  works on the main-thread run loop via `block_on_main`.
  This aims to be similar in API to `mac-notification-sys` to fit well into `notify-rust`.

### License

<sup>

`mac-usernotifications` is licensed under either of <a href="LICENSE-APACHE">Apache License, Version 2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.

</sup>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>
