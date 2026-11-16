# mac-usernotifications 0.3.1

Imported from the published crates.io archive, SHA-256
`08adbb1b9e11709f9ce628dd7471208e5dd3ad35a245dbc81e59a36dce3f3780`.
The archive records upstream commit
`5f63a95e317262761242efdde8abe1c1c4b0e819` in `.cargo_vcs_info.json`.
Upstream: <https://github.com/hoodie/mac-usernotifications>.

The archive omitted its license files. LICENSE-MIT and LICENSE-Apache were
retrieved from that exact upstream commit. Their Git blob IDs are
`e96dfb676ed9b6a150bd683896689eab4b3b6e6f` and
`8dada3edaf50dbc082c9a125058f25def75e625a`, respectively. The upstream license
remains MIT OR Apache-2.0.

Local runtime changes are confined to `src/lib.rs`, `src/delegate.rs`, and the
new `src/observer.rs`. The existing response API, polling, worker, permission,
category, presentation, and send code are unchanged. The observer receives
only responses constructed in the real delegate's routing function.

See [the caller and lifecycle design](../../../docs/NOTIFICATION_ACTIONS.md).
