# Mobile exploration

Proposal, September 5, 2026. Based on repository revision `88463a5` and the
vendor documentation linked below. This describes a direction and its validation
gates. No mobile binary or live Tailnet connection has been tested for this study.

Guaca mobile should connect to the same persistent workspace as desktop. Start
with self-hosted VPS access over Tailscale. Add managed workspace discovery at
the connection boundary later. Agents, schedules, coding jobs, plugin credentials
and approvals remain on `guacad`.

Use the existing React UI and a small Tauri mobile client as the first technical
spike. Validate iPhone first unless platform priorities change; preserve Android
as a target. Tauri already supports both, with Swift and Kotlin plugin support.
That establishes feasibility of the framework, not compatibility of this app.
[Tauri mobile prerequisites](https://v2.tauri.app/start/prerequisites/) and
[mobile plugins](https://v2.tauri.app/develop/plugins/develop-mobile/).

## The phone is where work is directed and unblocked

Three destinations, with one workspace selected at a time:

| Destination | First screen | Next action |
|---|---|---|
| Crews | Crew names, familiar characters, current activity and attention counts | Open a crew, then its wall or an agent's channel |
| Desk | Requests and escalations waiting on the operator | Read context, answer a question, allow or deny an action |
| Workspace | Current host, connection state, saved workspaces and settings | Switch workspace or inspect a connection problem |

Open on Crews, or restore the last conversation. A notification opens the
specific request after refreshing it. Incoming requests never take over a
conversation or steal the keyboard. A persistent Desk destination is a deliberate
mobile adaptation of the transient desktop desk; its badge disappears at zero.
An empty desk says that nobody is waiting. Completed work stays in the transcript.

The conversation takes the whole reading surface. The header names the crew and
agent, with a back control and an explicit details action. Tool activity remains
collapsed into chips. Keep the composer above the keyboard, support OS dictation,
and expose photos and files through the system picker. Preserve drafts per
workspace and channel. An attachment belongs to the message being composed,
never to whichever crew becomes selected while its upload finishes.

The agent inspector becomes a separate screen or sheet. Search is an explicit
action. Crew switching requires a tap; the desktop's pointer-proximity column
does not translate to touch. Preserve Guaca's paper/ink surfaces, character
geometry and restrained animation. Add named spacing and target-size tokens
through the existing CSS contract when implementation starts.

First scope: conversations, crew activity, questions, approvals, escalations,
stop, and attachments. Full repository setup, plugin administration and computer
viewers can follow. The API remains available; each omitted mobile surface needs
a clear route to desktop or the hosted web UI. A phone must still be able to read
the evidence required for an approval, including a diff or document.

## Tailnet access should be a supported installation path

Recommended deployment:

```text
Guaca on phone -> device Tailscale client -> VPS Tailscale Serve (HTTPS)
                                             -> 127.0.0.1:8787 -> guacad
```

Install Tailscale on the VPS and phone, join an authorized tailnet, and keep the
existing Compose loopback publication. On a VPS with Guaca already answering at
`127.0.0.1:8787`, the proposed setup is:

```sh
tailscale serve --bg http://127.0.0.1:8787
tailscale serve status
```

Use the full HTTPS hostname reported by Serve, such as
`https://guaca.example-tailnet.ts.net`. Serve provides tailnet-private access
and automatic TLS; it requires tailnet HTTPS to be enabled and respects access
rules. The example commands have been checked against documentation, not executed
against an operator's VPS. An installer must inspect existing Serve configuration
before changing it. [Serve](https://tailscale.com/docs/features/tailscale-serve)
and [CLI reference](https://tailscale.com/docs/reference/tailscale-cli/serve).

Use the installed Tailscale client for routing. Guaca needs no embedded VPN,
subnet router or exit node for this path. Keep normal HTTPS verification and
Guaca authentication. A private network does not turn every member into the
workspace operator. The full hostname also avoids relying on an HTTPS certificate
for a bare MagicDNS name or a `100.x` address. HTTPS setup publishes machine and
tailnet names in certificate transparency logs, which belongs in host setup.
[HTTPS configuration](https://tailscale.com/docs/how-to/set-up-https-certificates).

iOS Tailscale can connect on demand when a connection targets `*.ts.net`, once
the user enables that behavior. Guaca cannot assume it is enabled or silently
install the VPN profile. Other VPN use can affect the setting. This is the
closest path to opening Guaca without manually opening Tailscale each time.
[VPN On Demand](https://tailscale.com/docs/features/client/ios-vpn-on-demand).

“Out of the box” should mean: after Tailscale enrollment and host setup, open a
pairing invitation, confirm the named workspace, and connect. It cannot mean
access before network enrollment. Start with pasteable host and access key for a
technical spike. For a release, pair with a short-lived, single-use invitation
that exchanges for a revocable device credential. This pairing flow is new work.
The current fragment invitation contains the shared operator token.

Test HTTP commands, WSS, uploads, downloads, artifacts and the screen relay through
Serve. Test over cellular as well as Wi-Fi. Report DNS/network failure separately
from rejected credentials and incompatible servers where the platform permits
that distinction. A failed fetch alone does not prove Tailscale is off.

## The existing seams and the work they still need

| Evidence in the repository | Mobile consequence |
|---|---|
| `src/lib/transport.ts`: HTTP commands, WSS events, uploads and `probe` | Reuse the protocol. Add foreground/network recovery and explicit connection state. |
| `src/App.tsx`, `src/lib/store.ts`: reconnect calls `resynchronize` | Reuse durable refresh. Also refresh on resume, when a socket can appear open but be stale. |
| `src-tauri/src/server/mod.rs`: runtime snapshot plus live event feed | Preserve the snapshot/feed boundary. A phone must not infer current activity from missed deltas. |
| `src/lib/transport.ts`: `desktop` means `__TAURI_INTERNALS__` exists | This also identifies a mobile Tauri webview. Replace desktop assumptions with explicit client capabilities before sharing startup. |
| `src/components/HostSetup.tsx`: Docker-first native setup | Mobile connects to existing hosts. It must never enter local Docker setup. |
| `src-tauri/src/app.rs`, `src-tauri/Cargo.toml`: tray, Docker commands, broad runtime dependencies | Build a small mobile entry point and isolate native client dependencies. Do not compile the daemon into a phone as the shortcut. |
| `src/styles.css`: desktop layout, no narrow-screen media rules; Tauri window minimum width is 900 | Build actual phone navigation and keyboard behavior. Packaging the current page is insufficient. |
| `src/lib/transport.ts`: tokens stored in localStorage, one saved remote | Use OS-protected credential storage for native clients, workspace-scoped profiles, and device revocation. |
| Server CORS accepts specific Tauri origins; artifact framing has separate isolation | Verify actual iOS and Android origins, CORS, CSP and frames on devices. Do not widen access to opaque origins. |
| `/health` reports a build; `capabilities` reports host abilities | Add an explicit protocol compatibility policy before independent app-store and daemon release cycles. A commit hash alone is not a compatibility contract. |

Resuming must reconcile transcript, active runs, pending requests and selected
workspace before enabling a stale decision. Requests can expire or be answered
on desktop while the phone sleeps. Preserve server authority and current request
timeouts. Do not extend them to compensate for unreliable notification delivery.

Keep drafts locally. Do not queue approvals or blindly retry messages while
offline. A send interrupted after server acceptance has an unknown outcome;
safe retry needs a client request ID and server deduplication. Display that
uncertainty until it is reconciled. These concerns also improve remote desktop.

## Background delivery is separate work

Current notifications originate in the running frontend after it receives an
event. They cannot provide closed-app delivery. A native release needs a backend
notification path through APNs on iOS and the Android push provider. Apple's
model requires a provider server and device registration.
[Apple remote notifications](https://developer.apple.com/documentation/usernotifications/setting-up-a-remote-notification-server).

For the distributed native app, recommend an optional Guaca push relay. A
self-hosted daemon makes an outbound authenticated request to the relay; no
public inbound access to the VPS is required. The relay owns app push credentials.
Send generic notification text and opaque identifiers, not conversation content
or workspace credentials. Bind registration to the paired workspace and device,
support revocation, deduplicate notifications, and expire stale requests.

Opening a notification reconnects directly to the chosen workspace and fetches
the current request. Push delivery grants no authority, and actions should not
approve work from a lock-screen payload. Self-hosting remains usable without the
relay, with foreground updates and an explicit explanation of the notification
limit. The relay learns delivery metadata even with generic payloads.

A mobile web/PWA pilot is useful for validating layout and tailnet connectivity
before signing a binary. iOS Home Screen web apps support Web Push, but that
still requires server delivery and subscription work. It does not make the
current `new Notification()` implementation a background system.
[WebKit Web Push](https://webkit.org/blog/13878/web-push-for-web-apps-on-ios-and-ipados/).

## Managed hosting changes how a workspace is obtained

Self-hosted: pair with a host the operator supplies. Managed: sign in to Guaca,
select a space, and obtain its endpoint and scoped authorization from the service.
Both end in the same workspace client and HTTP/WSS protocol. Treat display name,
stable workspace identity, endpoint and credential reference as separate facts;
a managed endpoint can change without becoming a different workspace.

Do not require a Guaca account for basic self-hosted use. Keep managed ownership,
billing and discovery outside the runtime, consistent with [Hosting](HOSTING.md)
and [Account](ACCOUNT.md#managed-compute-is-the-next-phase). Never send a managed
account credential to an arbitrary pasted host. Switch connections atomically:
close the previous feed, clear its state, then load the next workspace. Scope
drafts, caches and notification destinations by workspace identity.

There is a current integration blocker: the documented `guaca-desktop` OAuth
client accepts loopback callbacks, not arbitrary Tailnet hostnames. A valid TLS
certificate and a reachable callback route do not change that registration.
Core VPS operation and account-backed Google sign-in are separate acceptance
tests. Do not promise the latter works automatically. Managed identity needs
its planned host-independent authorization flow; a list of customer Tailnet
callback exceptions is not that design. See [current account callbacks](ACCOUNT.md).

## Prove the first slice before committing to the shell

1. **Phone layout in the hosted UI.** At 320, 390 and 430 CSS pixels, navigate
   crews, read a conversation, type with the keyboard open and answer a request.
   Verify touch targets, text scaling, VoiceOver, scroll position and draft
   retention. Use fixtures, without production credentials or model spend.
2. **Real VPS over Serve.** Open the same workspace from a phone and desktop.
   Send once, attach a file, follow a live run, stop it, and answer one request.
   Confirm state agrees on both clients. Repeat over Wi-Fi and cellular.
3. **Tauri device spike.** Repeat that slice in a signed development app. Prove
   secure storage, system browser handoff, file/photo picking, keyboard behavior,
   artifact isolation and suspend/resume. Record native integration code needed.
   Keep Tauri if it passes; reopen the shell decision only on measured failures.
4. **Failure gate.** Disable Tailscale, rotate credentials, restart the daemon,
   switch networks, interrupt a send, and answer a phone request from desktop.
   Recover without duplicate work, stale approval, cross-workspace state or lost
   drafts. Record client/server versions and sanitized connection transitions.
5. **Release foundations.** Add device pairing/revocation, protocol compatibility
   and closed-app push. Validate stale/duplicate notification handling and offline
   notification taps. Managed discovery can then feed the same connection seam.

The next implementation should be the phone layout and connection lifecycle
slice. It tests the product on the existing server and produces work reusable
by either a native client or a Home Screen web app.
