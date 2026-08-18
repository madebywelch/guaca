# The Linux desktop an agent's computer boots. Built from the repository root,
# because it ships two files the app also compiles into itself:
#
#   docker build -f computer-image/Dockerfile -t guaca-computer:dev .
#
# `src-tauri/src/computer/desktop.rs` is the specification for this file, and
# `src-tauri/tests/apple.rs` checks the two against a real machine.
#
# Every comment here names a reason and points at `computer-image/README.md`,
# which holds it in full. That is not a style preference: Apple Container
# refuses a Dockerfile over 16 KiB outright, so the prose lives next door and
# `build.sh --check` fails this file over 15,000 bytes.

# Pinned by digest, never by tag, so two people get the same base. The same
# string is in `BASE_DIGEST`; `build.sh --digest` moves both. README, "The
# pinned base".
ARG BASE_IMAGE=debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241
FROM ${BASE_IMAGE}

LABEL org.opencontainers.image.title="Guaca computer" \
      org.opencontainers.image.description="The Linux desktop an agent's computer boots." \
      org.opencontainers.image.source="https://github.com/madebywelch/guaca" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"

# `--no-install-recommends` before it is a size choice: the XFCE metapackage
# recommends `xorg`, a second X server for a machine whose screen is Xvfb. One
# layer, because an apt cache in an earlier one is still in the image. README,
# "The packages, and why each group is here".
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      # The screen. `python3-websockify` is named although `novnc` reaches it:
      # Debian's `websockify` package ships no `websockify` binary at all.
      xvfb xfce4 xfce4-terminal x11vnc novnc websockify python3-websockify \
      # `startxfce4` launches through `dbus-launch` and loads resources with
      # `xrdb`. Recommendations of the XFCE packages rather than dependencies.
      dbus-x11 x11-xserver-utils \
      # One browser, and that is the design. README, "One browser, and every
      # click lands in it".
      chromium \
      # So the desktop is one a person watching over an agent's shoulder can
      # also use.
      thunar mousepad xdg-utils \
      # `browser.py` speaks DevTools over a WebSocket. Installed here so a
      # browse never pauses to fetch a Python package on a machine with no
      # network.
      python3 python3-websocket \
      # A screenshot, its conversion to JPEG, and the pointer and keyboard.
      scrot ffmpeg xdotool \
      # Or every screenshot of a page is a picture of empty boxes.
      fonts-dejavu-core \
      # What an agent fetches with and what `guaca-init` needs. `util-linux` is
      # named for `setsid`; `iproute2` is for the spike's measurements.
      curl git ca-certificates sudo procps psmisc util-linux iproute2 \
    && rm -rf /var/lib/apt/lists/*

# The account every command runs as, by number: the provider asks for uid 1000
# on every `exec` and PID 1 hands the volume to that number on every boot.
# Passwordless sudo because agents `apt-get install`, as they do on E2B.
RUN useradd --uid 1000 --create-home --home-dir /home/user --shell /bin/bash user \
    && printf '%s\n' 'user ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/guaca-user \
    && chmod 0440 /etc/sudoers.d/guaca-user

# `/opt/noVNC` is where `desktop.rs` serves the desktop from; Debian packages
# the same files elsewhere. The launcher is Guaca's own, written over the
# packaged one, which differs between releases. README, "The noVNC launcher".
COPY computer-image/novnc_proxy /usr/share/novnc/utils/novnc_proxy
RUN chmod 0755 /usr/share/novnc/utils/novnc_proxy && ln -s /usr/share/novnc /opt/noVNC

# One profile on this machine, reachable by every route to a browser. Here as
# well as in the home, because nothing has written the home's copy yet on a
# machine whose desktop has never started. README, "The Chrome flags are here
# twice on purpose".
COPY computer-image/google-chrome /usr/local/bin/google-chrome
RUN chmod 0755 /usr/local/bin/google-chrome \
    && ln -sf /usr/local/bin/google-chrome /usr/local/bin/google-chrome-stable

# The system's answer to "open this in a browser", which otherwise resolves to
# the packaged Chromium on a second profile. `--set` so an agent's own
# `apt-get install` cannot outbid it; `gnome-www-browser` only if a package
# already declares that group. README, "One browser, and every click lands in
# it".
RUN update-alternatives --install /usr/bin/x-www-browser x-www-browser \
      /usr/local/bin/google-chrome 200 \
    && update-alternatives --set x-www-browser /usr/local/bin/google-chrome \
    && if update-alternatives --list gnome-www-browser >/dev/null 2>&1; then \
         update-alternatives --install /usr/bin/gnome-www-browser gnome-www-browser \
           /usr/local/bin/google-chrome 200 \
         && update-alternatives --set gnome-www-browser /usr/local/bin/google-chrome; \
       fi

# The home the volume gets on its first boot. It cannot be built at
# `/home/user`, because the volume mounted there hides it; `guaca-init` copies
# this in without overwriting. README, "The home directory in the image".
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
    # The line `install_chrome_shim` appends, and the substring it looks for
    # before appending its own, so the two never stack up.
    && (grep -q '.local/bin' /opt/guaca/home/.profile 2>/dev/null \
        || printf '%s\n' 'PATH="$HOME/.local/bin:$PATH"' >> /opt/guaca/home/.profile)

# The icon and the menu read this one: an entry in the user's own XDG directory
# takes precedence over the packaged browser's.
COPY computer-image/google-chrome.desktop \
     /opt/guaca/home/.local/share/applications/google-chrome.desktop

# The same answer for the two routes a wrapper on PATH does not reach: `xdg-open`
# and Thunar read `mimeapps.list`, XFCE reads its preferred-application helper,
# and neither reads the other. Defaults rather than policy, since `guaca-init`
# copies with `cp -Rpn`. README, "One browser, and every click lands in it".
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
# last thing that writes into it: `cp -Rpn` applies the source directory's own
# attributes to the mount point, so a root-owned skeleton hands /home/user back
# to root on every boot, silently. README, "The home directory in the image".
RUN chown -R 1000:1000 /opt/guaca/home

# The driver and the session reader, as the app compiles them in. The runtime
# writes its own copy before using them, so these are for whatever runs first: a
# person on the desktop, and `sessions.py` on a machine that has never browsed.
COPY src-tauri/src/computer/browser.py /opt/guaca/browser.py
COPY src-tauri/src/computer/sessions.py /opt/guaca/sessions.py

COPY computer-image/guaca-init /usr/local/bin/guaca-init
RUN chmod 0755 /usr/local/bin/guaca-init

# Everything above assumed something about where a package put its files, or
# what an earlier instruction left behind. This is where those assumptions fail
# the build rather than the desktop: a moved `vnc.html` is a black rectangle in
# the viewer, and a missing `websocket` module is a browse that reports the tool
# is broken, both discovered by an operator rather than by CI.
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

# Set rather than inherited. A session that arrives without `HOME` resolves `~`
# somewhere the session reader does not look, which reads as an empty cookie jar
# on a machine that is signed in; `DISPLAY` is for an agent that types `xdotool`
# itself. README, "The environment".
ENV HOME=/home/user \
    LANG=C.UTF-8 \
    DISPLAY=:0 \
    GUAC_IDLE_SECONDS=900

# Deliberately no `VOLUME /home/user`: it would add an *anonymous* volume to any
# run that named none, and an anonymous volume carries no labels, so nothing
# here can see, claim or clean it up. README, "PID 1 is root".
WORKDIR /home/user

# And deliberately no `USER user`, which is a different question from who an
# agent's commands run as. Both halves were measured on Apple Container 1.2.2:
# the init process takes it and could then not prepare the volume, and `exec`
# ignores it, so `apple.rs` passes `--uid 1000 --gid 1000` itself. README,
# "PID 1 is root".

# Deliberately not wrapped in an init: this process exiting is how the machine
# stops itself. See the comment at the top of `guaca-init`.
ENTRYPOINT ["/usr/local/bin/guaca-init"]
