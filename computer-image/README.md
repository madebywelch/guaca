# The desktop image an agent's computer boots

A Debian with a screen on it: Xvfb, XFCE, x11vnc, noVNC, Chromium, Python, and
a watchdog as PID 1. `src-tauri/src/computer/desktop.rs` is the specification —
every command that file runs on a machine has to work here, unchanged, whether
the machine is an E2B sandbox or a container on the operator's own Mac. That is
the whole point of the boundary: the desktop, the browser, screenshots and
sign-in detection are commands, so a provider that can create a machine and run
a command gets all of them for free, and only if the machines look alike.

## Building one, and using it

```sh
computer-image/build.sh --check     # everything checkable without a builder
computer-image/build.sh             # build for this machine
export GUAC_COMPUTER_IMAGE=guaca-computer:dev
```

`build.sh` uses Apple's `container build` when `container` is on `PATH` and
`docker buildx` otherwise. The build context is the repository root, not this
directory, because the image ships `browser.py` and `sessions.py` from
`src-tauri/src/computer/` — the same two files the app compiles into itself.

`GUAC_COMPUTER_IMAGE` is read at startup and shown in Settings when it is set.
It exists so a reviewer can try the feature before the image is published; it is
not a user setting, and an app running one is running something other than the
image the release was tested with.

## What is in here

| file | what it is |
| --- | --- |
| `Dockerfile` | the image. Its final `RUN` asserts every path the app depends on, so a package that moves its files fails the build instead of the desktop |
| `guaca-init` | PID 1: prepares the home volume, then watches a heartbeat file |
| `google-chrome` | the wrapper that puts every route to a browser on one profile |
| `google-chrome.desktop` | the entry the desktop icon and the menu read |
| `novnc_proxy` | the launcher `desktop.rs` calls, installed over Debian's |
| `build.sh` | build, and the checks that need no builder |
| `BASE_DIGEST` | the Debian base, pinned |
| `IMAGE_REF` | what the app pulls. One line, no comment: `image.rs` includes it verbatim |

## Three things about it that are not obvious

**PID 1 has to be able to exit.** A container stops when its first process
returns, and stopping is what this image's watchdog is for: Guaca touches
`/run/guaca/heartbeat` on every running local machine, and when Guaca is
force-quit or crashes, nothing touches it and the machine puts itself to sleep
after the operator's idle period. That is why there is a shell loop here and not
systemd, s6 or tini, and why `container create` is called without `--init`. A
supervisor would outlive the watchdog that had already decided the machine
should stop.

**The home directory in the image is at `/opt/guaca/home`.** Each agent gets a
named volume mounted at `/home/user`, and a volume hides whatever the image put
underneath it. So the skeleton — `.profile`, the Chrome wrapper, the desktop
entry — is built somewhere else and copied in on every boot without overwriting
anything already there. The same boot chowns what it copied, because a fresh
volume's root belongs to root, and removes Chrome's `SingletonLock`, which a
stopped container leaves behind and which would otherwise make the next Chrome
refuse the profile as "in use". `guaca-init` reaches for `sudo` when it is not
root, because whether the image's `USER` applies to PID 1 or only to `exec`
sessions is the runtime's decision and not this file's.

**The Chrome flags are here twice on purpose, and must not disagree.** The app
writes its own copy of the wrapper into `~/.local/bin` on every desktop start;
the image ships one as well, because on a machine whose desktop has never
started there is still a `google-chrome` on `PATH`, a desktop icon and a file
association. A sign-in performed through any of those before the first desktop
start would land in a second profile — which is not an error anybody sees. It is
an operator signing in to a browser no agent can use, and detection truthfully
reporting an empty jar. `build.sh --check` compares the flags in this directory
against the ones in `desktop.rs` and fails when they drift.

## The pinned base

`BASE_DIGEST` and the Dockerfile's `ARG BASE_IMAGE` both name
`debian:bookworm-slim` by digest, and `build.sh --check` fails when the two
disagree. The digest was read from `registry-1.docker.io` on 2026-08-18; it is
the multi-platform index, so it covers both platforms the workflow publishes.

The trade is that a pinned base does not pick up Debian's security updates on
its own. `build.sh --digest` re-reads the tag from the registry and rewrites
both files, which is a commit and a review rather than a silent change in what
every operator's agents boot. That is the same reason the app pulls a digest
rather than a tag: two people on one release must not get different machines.

## Publishing

`.github/workflows/computer-image.yml` builds `linux/arm64` and `linux/amd64`,
pushes them to `ghcr.io/<owner>/guaca-computer`, and opens a pull request that
rewrites `IMAGE_REF` to the digest it just pushed.

It has never run. This repository has no hosted CI at all — that workflow is
the first — and it publishes under a namespace only the maintainer can write
to, so a first publish needs them to:

1. merge the workflow to `main`;
2. run it once from the Actions tab (`workflow_dispatch` is there for exactly
   this: nothing pushes to `computer-image/` after the workflow lands);
3. allow GitHub Actions to create pull requests, in the repository's Actions
   settings, or the final step fails after the image is already published;
4. merge the pull request it opens, which is the moment the app starts pulling
   a real image.

Until then `IMAGE_REF` names `0.1.0-unpublished`, nothing pulls, and
`GUAC_COMPUTER_IMAGE` is the way to see the feature work.

## Proving it works

The image was written on a Mac with no Apple Container and no running Docker,
so nothing in it has been built or booted. What replaces having tried it is:

- the Dockerfile's own assertions, which fail the build if a path the app uses
  is not where this file thinks it is;
- `build.sh --check`, which is `sh -n` on every script, a dry run of the noVNC
  launcher's argument handling, and the drift checks against `desktop.rs`;
- `src-tauri/tests/apple.rs`, ten `#[ignore]`d tests that are the ten smoke
  items in `docs/LOCAL_COMPUTERS.md`, run against a real machine by
  `scripts/spike-apple.sh`.

```sh
./scripts/spike-apple.sh
```

That script also prints what the tests cannot assert — the raw text of
`container --version`, where labels land in `container ls --format json`, what
the runtime says when a name is already taken — because those are the guesses
marked in `src-tauri/src/computer/apple.rs`, and asserting a guess only proves
it was guessed twice.
