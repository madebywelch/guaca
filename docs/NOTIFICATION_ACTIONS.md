# Native notification actions

## Caller and lifetime

The app installs one native-response subscription during setup, before a
notification can be sent. Each app session generates an unpredictable UUID
namespace. A clickable request's identifier is that namespace followed by its
existing frontend click number. Sending uses `mac_usernotifications::send`,
which reports acceptance without registering a response waiter.

```rust,ignore
let subscription = observe_responses(move |response| {
    // Only this session's IDs and real native default/dismiss actions qualify.
    // Emit notification-clicked or notification-dismissed with the click number.
});
send(notification.id(&session_request_id)).await?;
// On actual exit, outside any response callback:
subscription.detach().wait();
```

The observer is invoked only by the existing native delegate. A delivered-list
poll, inferred disappearance, or timeout cannot invoke it. The dependency's
existing `response()` API remains unchanged for other callers.

The frontend's existing 64-entry map remains the consumption authority. A
default action deletes the entry before navigating. A genuine dismiss deletes
it without navigating. Later duplicates, including default after dismiss,
cannot navigate. Silent disappearance does neither. Rejected sending removes
the frontend entry as before. Old-session and malformed request IDs are ignored.
No notification body, reply, or credential enters these events.

## Observer API

```rust,ignore
pub fn observe_responses(
    callback: impl Fn(&NotificationResponse) + Send + Sync + 'static,
) -> Option<ResponseSubscription>;

impl ResponseSubscription {
    pub fn detach(self) -> ResponseDrain;
}

impl ResponseDrain {
    pub fn wait(self);
}
```

Registration fails if another subscription is active. Dispatch acquires an
invocation lease only while the subscription is active, then calls the observer
without holding the registry or lifecycle lock. Reentrant detachment is safe.
Dropping the subscription detaches without waiting. Detachment prevents new
leases; an already leased callback may still finish. The returned drain waits
for those leases, so after `wait()` returns no callback from that subscription
can run. Waiting inside its own callback is invalid because it would join itself.
The app drains only on actual exit, not an exit request that may be prevented.

One subscription and its active callback leases are the only native observer
state. There are no per-banner tasks or registrations to grow when silent
dismissals omit a callback. The frontend retention bound is unchanged.

## Scope and alternatives

The vendored source is mac-usernotifications 0.3.1. The extension adds the
observer and one delegate dispatch hook; permission, category, presentation,
worker, scheduling, and existing response-future behavior stay upstream.
The app does not replace the process delegate. The host's delivery, presence,
offer, claim, and release policies are untouched.

An action button would alter visible behavior. A timeout or grace period would
only guess when a click becomes impossible. Waiting on the existing future
cannot work because an inferred dismissal consumes it. Replacing the complete
native adapter would take over unrelated permission and presentation behavior.

## Verification contract

The pristine-source assay preserves the behavioral counterexample: polling can
finish before the default callback and cancel its response receiver. The
amended delegate dispatch is exercised with both event orders, native dismissal,
duplicates, observer replacement/detachment, and in-flight shutdown. Frontend
tests exercise the actual event listeners and map, including rejection and
eviction. A synthetic disappearance must never consume the callback, and a
native default must navigate exactly once while its map entry is retained.
These schedules prove a mechanism, not the cause of an earlier untraced failure.

Run `python3 scripts/test-notification-actions.py` on macOS. It verifies the
published archive checksum and unchanged upstream files, controls only the
external delivered-list completion, and runs the real 500 ms polling loop.
The pristine implementation must fail the desired click-preservation assertion;
the amended dispatch must pass both orders. Standalone tests use the app's
locked runtime dependency versions without the upstream examples' dev dependencies.
The normal Rust gate runs this check on macOS.

### Existing test change

| Test | Changed assertion | Reason |
| --- | --- | --- |
| `notify.test.ts`: listens for clicks before it hands anything over, and lands on the channel clicked | Listener count: 1 to 2; additionally checks exact event names and both registrations before sending | A real native dismissal needs its own handler. Navigation and duplicate assertions are unchanged. |
