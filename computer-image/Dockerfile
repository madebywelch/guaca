# The Linux desktop an agent's computer boots.
#
# Built from the repository root, because it ships two files the app also
# compiles into itself:
#
#   docker build -f computer-image/Dockerfile -t guaca-computer:dev .
#
# `computer-image/build.sh` is that command plus the checks worth running
# first. Every command `src-tauri/src/computer/desktop.rs` runs on a machine has
# to work here — that file is the specification for this one, and
# `src-tauri/tests/apple.rs` is where the two are checked against each other on
# a real machine.

# Pinned by digest, never by tag: two people building `debian:bookworm-slim` a
# week apart get different bases, and a machine an operator cannot reproduce is
# one nobody can debug. This is the multi-platform index for `bookworm-slim`,
# read from registry-1.docker.io on 2026-08-18; it carries linux/amd64 and
# linux/arm64/v8, which are the two platforms the workflow publishes.
#
# The same string is in `computer-image/BASE_DIGEST`, which is what `build.sh`
# and the publishing workflow pass in, and `build.sh --check` fails when the two
# disagree. `build.sh --digest` moves both forward.
ARG BASE_IMAGE=debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241
FROM ${BASE_IMAGE}

LABEL org.opencontainers.image.title="Guaca computer" \
      org.opencontainers.image.description="The Linux desktop an agent's computer boots." \
      org.opencontainers.image.source="https://github.com/madebywelch/guaca" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"

# `--no-install-recommends` is a behavioural choice before it is a size one.
# The XFCE metapackage recommends `xorg`: a real X server and a set of drivers
# for hardware this machine does not have. The screen here is Xvfb, and a second
# server for the same display is not something to install by accident. What the
# recommendations would otherwise have brought and this image does need is named
# explicitly below.
#
# One layer, because an apt cache in an earlier layer is still in the image
# however thoroughly a later one deletes it.
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      # The screen: a framebuffer, a session, a VNC server, and the bridge that
      # puts it in a browser. `desktop.rs::start_desktop` starts exactly these.
      #
      # `python3-websockify` is named although `novnc` reaches it anyway, and
      # the reason is worth writing down: Debian's `websockify` package is the C
      # `rebind` helper and ships no `websockify` binary at all. The proxy the
      # desktop actually runs is in `python3-websockify`, two dependency edges
      # away from `novnc`, and a package that stopped depending on it would take
      # the desktop's only bridge with it.
      xvfb xfce4 xfce4-terminal x11vnc novnc websockify python3-websockify \
      # `startxfce4` launches the session through `dbus-launch`, and `xrdb` is
      # what it loads the session's resources with. Both are recommendations of
      # the XFCE packages rather than dependencies.
      dbus-x11 x11-xserver-utils \
      # The browser, singular, and that is the whole design. Everything that
      # opens a page — `browse`, the desktop icon, a link clicked in the file
      # manager — is shimmed onto one Chromium profile, because that profile is
      # the only one `browse` can drive and the only one sign-in detection
      # reads. The prompt tells agents the same thing in words. A second browser
      # is 250 MB whose only use is a sign-in nobody can see.
      chromium \
      # A file manager and an editor, so the desktop is one a person watching
      # over an agent's shoulder can also use.
      thunar mousepad xdg-utils \
      # `browser.py` speaks the DevTools protocol over a WebSocket. E2B installs
      # this client on the first browse; a machine on this Mac may have no
      # network at all, and a browse that pauses to fetch a Python package is a
      # browse that fails on an aeroplane.
      python3 python3-websocket \
      # A screenshot, its conversion to JPEG, and the pointer and keyboard.
      scrot ffmpeg xdotool \
      # Text has to have something to be drawn in, or every screenshot of a page
      # is a picture of empty boxes.
      fonts-dejavu-core \
      # What an agent needs to fetch things and what `guaca-init` needs to
      # prepare a volume it does not own. `util-linux` is named for `setsid`,
      # which detaches every long-lived process the desktop starts and is in the
      # base image already — named anyway, because a base that dropped it would
      # be a desktop that dies with the shell that started it. `iproute2` is for
      # the spike's network measurements, taken from inside the guest.
      curl git ca-certificates sudo procps psmisc util-linux iproute2 \
    && rm -rf /var/lib/apt/lists/*

# The account every command runs as. The number matters more than the name: the
# provider asks for uid 1000 by number on every `exec`, and PID 1 hands the home
# volume to that number on every boot, so a uid that moved between image
# versions would lock an agent out of its own disk. Passwordless sudo because
# agents `apt-get install`, which is a thing the E2B machines let them do too.
RUN useradd --uid 1000 --create-home --home-dir /home/user --shell /bin/bash user \
    && printf '%s\n' 'user ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/guaca-user \
    && chmod 0440 /etc/sudoers.d/guaca-user

# `/opt/noVNC` is the path `desktop.rs` serves the desktop from, and Debian
# packages the same files at `/usr/share/novnc`. The launcher is Guaca's own,
# written over whatever the distribution shipped: Debian's differs between
# releases, and one command that behaves the same on every image is worth more
# than the packaged variant.
COPY computer-image/novnc_proxy /usr/share/novnc/utils/novnc_proxy
RUN chmod 0755 /usr/share/novnc/utils/novnc_proxy && ln -s /usr/share/novnc /opt/noVNC

# One profile on this machine, reachable by every route to a browser. The
# wrapper is the one `desktop.rs::install_chrome_shim` writes into the home
# directory on every desktop start; it is here as well because nothing has
# written that yet on a machine whose desktop has never started, and a sign-in
# performed in that window would land in a profile no agent can drive.
#
# `/usr/local/bin` comes before `/usr/bin` on PATH, so this shadows a
# `google-chrome` typed into a shell. The `-stable` link goes here rather than
# in `/usr/bin`, where the wrapper looks for the real browser: a link there
# would point the wrapper back at itself.
COPY computer-image/google-chrome /usr/local/bin/google-chrome
RUN chmod 0755 /usr/local/bin/google-chrome \
    && ln -sf /usr/local/bin/google-chrome /usr/local/bin/google-chrome-stable

# The system's answer to "open this in a browser". `x-www-browser` is what
# Debian's own tools and `sensible-browser` follow, and Chromium claims it when
# it is installed; left alone it points at `/usr/bin/chromium`, which is the
# real browser without the flags, so a link opened that way starts a second
# profile on the machine — the failure `install_chrome_shim` exists to prevent,
# arriving through a path that file does not cover.
#
# `--set` as well as `--install`, so the choice is manual: an `apt-get install`
# an agent runs cannot outbid it with a higher priority. `gnome-www-browser`
# only if some package already declares it, because registering a group nothing
# reads would put a link on the system for a name no program here looks up.
RUN update-alternatives --install /usr/bin/x-www-browser x-www-browser \
      /usr/local/bin/google-chrome 200 \
    && update-alternatives --set x-www-browser /usr/local/bin/google-chrome \
    && if update-alternatives --list gnome-www-browser >/dev/null 2>&1; then \
         update-alternatives --install /usr/bin/gnome-www-browser gnome-www-browser \
           /usr/local/bin/google-chrome 200 \
         && update-alternatives --set gnome-www-browser /usr/local/bin/google-chrome; \
       fi

# The home directory the volume gets on its first boot. It cannot be built at
# `/home/user`, because the volume mounted there hides everything the image put
# underneath it; `guaca-init` copies this in without overwriting.
RUN mkdir -p /opt/guaca/home/.local/bin \
             /opt/guaca/home/.local/share/applications \
             /opt/guaca/home/.guac/chrome \
             /opt/guaca/home/inbox \
    && cp -a /etc/skel/. /opt/guaca/home/ \
    && cp /usr/local/bin/google-chrome /opt/guaca/home/.local/bin/google-chrome \
    && chmod 0755 /opt/guaca/home/.local/bin/google-chrome \
    # An absolute link to where this will be, not to where it is being built.
    && ln -sf /home/user/.local/bin/google-chrome \
              /opt/guaca/home/.local/bin/google-chrome-stable \
    # The same line `install_chrome_shim` appends, and it looks for this exact
    # substring before appending its own, so the two never stack up.
    && (grep -q '.local/bin' /opt/guaca/home/.profile 2>/dev/null \
        || printf '%s\n' 'PATH="$HOME/.local/bin:$PATH"' >> /opt/guaca/home/.profile)

# The icon and the menu entry read this one: a desktop entry in the user's own
# XDG directory takes precedence over the packaged browser's.
COPY computer-image/google-chrome.desktop \
     /opt/guaca/home/.local/share/applications/google-chrome.desktop

# And the desktop's own answer to the same question, in the two places a click
# is resolved. `x-www-browser` above covers a program that asks the system;
# these cover a person and an agent using the session: `xdg-open` and Thunar
# read `mimeapps.list`, and XFCE's menu, terminal and "Open Link" read its
# preferred-application helper, which consults neither. Each route that is not
# named here opens the packaged Chromium on its own profile, and a sign-in
# performed there is invisible to `browse` and to detection, with nothing
# reporting an error — the reasoning in `desktop.rs::install_chrome_shim`,
# applied to the routes a wrapper on PATH does not intercept.
#
# `google-chrome.desktop` is the entry beside this one, in the user's own XDG
# directory, so the association resolves to the shim rather than to the browser.
# The helper names the image's copy of the wrapper, which is on every machine
# whether or not a desktop has ever started.
#
# `guaca-init` copies the skeleton with `cp -Rpn`, so these are defaults on a
# first boot and an agent that changes its own browser keeps the change.
RUN mkdir -p /opt/guaca/home/.config/xfce4 \
             /opt/guaca/home/.local/share/xfce4/helpers \
    && printf '%s\n' \
       '[Default Applications]' \
       'text/html=google-chrome.desktop' \
       'x-scheme-handler/http=google-chrome.desktop' \
       'x-scheme-handler/https=google-chrome.desktop' \
       'x-scheme-handler/about=google-chrome.desktop' \
       'x-scheme-handler/unknown=google-chrome.desktop' \
       > /opt/guaca/home/.config/mimeapps.list \
    && printf '%s\n' 'WebBrowser=google-chrome' \
       > /opt/guaca/home/.config/xfce4/helpers.rc \
    && printf '%s\n' \
       '[Desktop Entry]' \
       'Version=1.0' \
       'Type=X-XFCE-Helper' \
       'X-XFCE-Category=WebBrowser' \
       'X-XFCE-CommandsWithParameter=/usr/local/bin/google-chrome "%s"' \
       'X-XFCE-Commands=/usr/local/bin/google-chrome' \
       'Name=Google Chrome' \
       'Icon=google-chrome' \
       > /opt/guaca/home/.local/share/xfce4/helpers/google-chrome.desktop

# The skeleton belongs to the account it is copied for, and this must be the
# last thing that writes into it.
#
# `guaca-init` copies it with `cp -Rpn`, which preserves what it finds — and,
# because the source is `.../home/.`, applies the *source directory's* own
# attributes to the destination directory as well. A root-owned skeleton
# therefore hands the volume's mount point back to root every boot, which on a
# live machine left /home/user unwritable while every command reported success.
# `guaca-init` chowns the mount point after the copy for the same reason. Both,
# because the failure is silent and shows up much later as an agent that cannot
# write its own home.
RUN chown -R 1000:1000 /opt/guaca/home

# The driver and the session reader, exactly as the app compiles them in. The
# runtime writes its own copy of each before it uses them, so these are for
# whatever runs before that: a person on the desktop, and `sessions.py` asked
# about a machine that has never browsed.
COPY src-tauri/src/computer/browser.py /opt/guaca/browser.py
COPY src-tauri/src/computer/sessions.py /opt/guaca/sessions.py

COPY computer-image/guaca-init /usr/local/bin/guaca-init
RUN chmod 0755 /usr/local/bin/guaca-init

# Everything above assumed something about where a package put its files. This
# is where those assumptions fail the build rather than the desktop: a moved
# `vnc.html` is a black rectangle in the viewer, and a missing `websocket`
# module is a browse that reports the tool is broken, both of them discovered by
# an operator rather than by CI.
RUN set -eu; \
    test -f /usr/share/novnc/vnc.html; \
    test -x /opt/noVNC/utils/novnc_proxy; \
    test -f /opt/noVNC/vnc.html; \
    test -x /usr/bin/websockify; \
    test -x /usr/bin/Xvfb; \
    test -x /usr/bin/x11vnc; \
    test -x /usr/bin/startxfce4; \
    test -x /usr/bin/chromium; \
    test -x /usr/bin/scrot; \
    test -x /usr/bin/ffmpeg; \
    test -x /usr/bin/xdotool; \
    test -x /usr/bin/setsid; \
    test -x /usr/bin/pgrep; \
    test -x /usr/bin/pkill; \
    test -x /usr/bin/base64; \
    test -x /usr/bin/sudo; \
    test -x /usr/local/bin/guaca-init; \
    test -x /usr/local/bin/google-chrome; \
    test "$(readlink -f /usr/local/bin/google-chrome-stable)" = /usr/local/bin/google-chrome; \
    test "$(readlink -f /etc/alternatives/x-www-browser)" = /usr/local/bin/google-chrome; \
    ! test -x /usr/bin/firefox-esr; \
    python3 -c 'import websocket'; \
    python3 -c 'import sqlite3'; \
    sh -n /usr/local/bin/guaca-init; \
    sh -n /opt/noVNC/utils/novnc_proxy; \
    test "$(stat -c %u /opt/guaca/home)" = 1000; \
    test "$(stat -c %u /opt/guaca/home/.local/bin/google-chrome)" = 1000; \
    grep -q 'user-data-dir=/home/user/.guac/chrome' /usr/local/bin/google-chrome; \
    grep -q '.local/bin' /opt/guaca/home/.profile; \
    grep -q 'x-scheme-handler/https=google-chrome.desktop' /opt/guaca/home/.config/mimeapps.list; \
    grep -q 'WebBrowser=google-chrome' /opt/guaca/home/.config/xfce4/helpers.rc; \
    grep -q '^X-XFCE-Commands=/usr/local/bin/google-chrome$' \
      /opt/guaca/home/.local/share/xfce4/helpers/google-chrome.desktop; \
    test "$(stat -c %u /opt/guaca/home/.config/mimeapps.list)" = 1000

# `HOME` is set rather than inherited: a command session that arrives without
# one resolves `~` somewhere else, and the browser driver that Guaca writes to
# `~/.guac` would land in a directory the session reader does not look in. That
# failure is not an error — it is a machine reporting an empty cookie jar while
# signed in.
#
# `DISPLAY` is the screen every graphical command lands on. `desktop.rs` names
# it on each command anyway; this is for an agent that types `xdotool` itself
# and would otherwise be told there is no display.
#
# `GUAC_IDLE_SECONDS` is how long a machine nobody is using stays up. The
# provider overrides it per container with the operator's setting; this is what
# the image does when somebody runs it by hand.
ENV HOME=/home/user \
    LANG=C.UTF-8 \
    DISPLAY=:0 \
    GUAC_IDLE_SECONDS=900

# Deliberately no `VOLUME /home/user`. The provider always mounts a named
# volume there, and every named volume it makes carries the three labels that
# say whose it is. A `VOLUME` line adds an *anonymous* volume whenever somebody
# runs this image without naming one, and an anonymous volume carries no labels
# at all: `list_owned` cannot see it, the sweep cannot claim it, and it holds a
# home directory's worth of disk until a person finds it by hand. The
# declaration buys nothing the `--mount` does not already give.
WORKDIR /home/user

# And deliberately no `USER user`, which is not the same question as who an
# agent's commands run as. Both halves were measured on Apple Container 1.2.2:
#
# - The init process *does* take the image's `USER`. With `USER user` it was
#   uid 1000, and a fresh volume's root belongs to root, so PID 1 could not
#   write the home at all: the skeleton never arrived, XFCE could not write its
#   own config, and three of the ten conformance tests failed on `Permission
#   denied` in `/home/user`. Preparing that volume is root's work and has to
#   happen before anything else runs.
# - `exec` does *not* take it. It ran as uid 0 whatever this line said, which
#   is why `apple.rs` passes `--uid 1000 --gid 1000` explicitly. An agent's
#   commands are the unprivileged account because the provider says so, not
#   because the image does.
#
# So PID 1 is root, it hands the home to uid 1000 on every boot, and nothing an
# agent runs is root unless the agent reaches for `sudo`.

# Deliberately not wrapped in an init: this process exiting is how the machine
# stops itself. See the comment at the top of `guaca-init`.
ENTRYPOINT ["/usr/local/bin/guaca-init"]
