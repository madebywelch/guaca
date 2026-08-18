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
| `Dockerfile.dockerignore` | what the build context leaves out. A short exclude list, never `*` plus `!` and never over ~1.9 KB: Apple's builder refuses both, unhelpfully |
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
anything already there. The same boot chowns what it copied, and removes
Chrome's `SingletonLock`, which a stopped container leaves behind and which
would otherwise make the next Chrome refuse the profile as "in use".

**A stopped container keeps its writable layer, and two locks in it will stop a
machine coming back.** That is measured on 1.2.2, not assumed: a file written to
`/tmp` survives a stop and a start. `SingletonLock` is one; the other is
`/tmp/.X0-lock`, which Xvfb refuses to start over, so the second boot of a
machine had no display at all and the browser died on "Missing X server or
$DISPLAY" — a desktop that never comes back after a wake, which reads as a
machine that lost its session rather than as a lock file. `guaca-init` removes
both on every boot, and the socket directory beside the display lock, which is
the same lock by another name. The provider also gives the container a tmpfs at
`/tmp`, which makes the X half moot there; the image clears it anyway, because
the image is not only ever run by that provider.

**PID 1 is root, and that is not a statement about who agents are.** The image
sets no `USER`, so the init process is root and can hand a freshly created
volume — an empty filesystem owned by root — to uid 1000 before anything else
runs. Agents are still unprivileged: `apple.rs` passes `--uid 1000 --gid 1000`
on every `exec`, which Apple Container 1.2.2 requires anyway, because `exec`
there ran as uid 0 whatever the image's `USER` said. The version with `USER
user` was measured and failed: PID 1 could not write `/home/user` at all, so
the skeleton never arrived, XFCE could not save its config, and three
conformance tests failed on `Permission denied` a long way from the cause.

There is deliberately no `VOLUME /home/user` in the Dockerfile. The provider
names its volume and labels it; a `VOLUME` line would add an *anonymous* one
whenever the image is run without a mount, and an anonymous volume carries no
labels, so nothing in this app can see it, claim it or clean it up.

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

Everything here was written on a Mac with no Apple Container and no running
Docker, from Debian's package lists and from `desktop.rs`. It has since been
built: Apple Container 1.2.2 builds this Dockerfile, and every assertion in its
final `RUN` passes. That settles the half about where packages put their files —
`vnc.html`, `websockify`, `Xvfb`, `startxfce4`, the `websocket` module — which
is the half that would otherwise have been discovered by an operator looking at
a black rectangle.

### What that build taught about `Dockerfile.dockerignore`

Three things, all of them enforced by `build.sh --check` rather than remembered,
and all of them found by building rather than by reading documentation. This is
where the reasoning lives, because two of the three are about the size of that
file and the third is what happens when someone writes an explanation into it.

**Apple's builder does read it.** The context transfer drops to 45 B, so the
per-Dockerfile form — `<dockerfile>.dockerignore`, consulted ahead of any
`.dockerignore` at the context root — was the right choice. A root one is not
merely redundant here: `Dockerfile.ci` does `COPY . .` and needs the whole
checkout, so a root ignore file excluding everything would starve it.

**It does not accept `*` plus `!` re-inclusions**, which is the idiom Docker's
own documentation uses and what this file was first written as. The build fails
during context transfer with

```text
#4 ERROR: changes out of order: "computer-image/google-chrome" ""
```

before any instruction runs, naming a file that was let back in rather than the
pattern that let it. So the file excludes and never re-includes — which is why
`src-tauri/src` is absent from it, since the image copies `browser.py` and
`sessions.py` out of that directory and there is no way to name them back in.

**It must stay under about 1.9 KB.** Above that, `container build` ends
immediately with

```text
Error: unavailable: "Stream unexpectedly closed."
```

which says nothing about ignore files at all. Bisected on a live runtime and
content-independent: 1938 bytes builds, 2230 bytes does not. Apple's builder
shim appears to cap that transfer around 2 KB. So the file holds its entries and
a pointer here, `--check` fails it over 1500 bytes, and every reason for what is
in it is in this section instead.

The cost of an exclude list, stated once: a new large directory at the
repository root is in the context until somebody adds it to that file.

What the image does once it boots is still the spike's question, and the
answers to it are:

- the Dockerfile's own assertions, which fail the build if a path the app uses
  is not where this file thinks it is;
- `build.sh --check`, which is `sh -n` on every script, a dry run of the noVNC
  launcher's argument handling, the drift checks against `desktop.rs`, and the
  build context;
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
