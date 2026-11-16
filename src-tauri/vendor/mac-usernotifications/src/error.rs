/// Errors returned by this crate.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// No bundle identifier (need valid `.app` bundle).
    #[error("No bundle identifier found. UNUserNotificationCenter requires a valid .app bundle.")]
    NoBundleIdentifier,

    /// macOS rejected the request.
    #[error("macOS rejected the notification request")]
    NotificationRejected,

    /// Delivery was interrupted.
    #[error("Response delivery was interrupted")]
    ResponseDeliveryInterupted(#[from] futures_channel::oneshot::Canceled),

    /// Mainthread not running.
    #[error("Mainthread not running")]
    MainThreadNotRunning,
}
