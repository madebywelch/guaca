# syntax=docker/dockerfile:1.7
#
# The daemon, as an image.
#
# One image, wherever it runs: a box somebody rents, one guaca.ai hands them,
# or a container on a laptop. The desktop app is not in here and cannot be
# (docs/ARCHITECTURE.md, *Why there is no Docker image for the app*); this is
# the second host over the same library, built without Tauri and serving the
# same `dist/` the desktop embeds.
#
#   docker build --build-arg GUACA_COMMIT=$(git rev-parse --short=7 HEAD) -t guacad .
#   docker run --rm -p 127.0.0.1:8787:8787 -v guaca:/var/lib/guaca guacad
#
# `scripts/image.sh` builds it and proves it answers. The commit is an argument
# because `.git` is not in the build context, and the same string has to reach
# `/health` and the page's About so a box and a laptop can be told apart.

ARG GUACA_COMMIT=""

# ---- the page ---------------------------------------------------------------
FROM node:22-bookworm-slim AS web
WORKDIR /app
RUN corepack enable
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY index.html tsconfig.json vite.config.ts ./
COPY src ./src
ARG GUACA_COMMIT
ENV GUACA_COMMIT=$GUACA_COMMIT
# The same `pnpm build` CI runs: the typecheck is the gate, not a nicety.
RUN pnpm build

# ---- the daemon -------------------------------------------------------------
FROM rust:1.95-bookworm AS daemon
WORKDIR /app/src-tauri
# Dependencies first, so a source-only change reuses the compiled tree. The
# stubs stand in for every target the manifest names; build.rs is stubbed
# because the real one only does anything with the desktop feature on.
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./
RUN mkdir -p src/bin \
 && echo 'fn main() {}' > src/main.rs \
 && echo 'fn main() {}' > src/bin/guacad.rs \
 && echo '' > src/lib.rs \
 && echo 'fn main() {}' > build.rs \
 && cargo build --release --no-default-features --features server --bin guacad \
 && rm -rf src build.rs target/release/deps/guac* target/release/deps/libguac*
COPY src-tauri/ ./
ARG GUACA_COMMIT
ENV GUACA_COMMIT=$GUACA_COMMIT
RUN cargo build --release --no-default-features --features server --bin guacad

# ---- what runs --------------------------------------------------------------
FROM debian:bookworm-slim
# `curl` is the health check. TLS roots are for the model endpoints, the
# sandboxes and the plugins the daemon calls out to. `git` and `gh` are what a
# remote-linked repository is cloned, fetched and pushed with, and `claude` is
# the coding harness, which spends ANTHROPIC_API_KEY when the operator sets it
# (a plan cannot be signed in to here, and is not offered).
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git \
 && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
      -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
 && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
      > /etc/apt/sources.list.d/github-cli.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends gh \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1000 --home-dir /var/lib/guaca --create-home guaca
# Claude Code's own installer, then the binary moved where every user finds it.
# The updater is off: a container's version is its image's version, and a
# binary that replaced itself would vanish on the next start anyway.
RUN curl -fsSL https://claude.ai/install.sh | bash \
 && install -m 755 /root/.local/bin/claude /usr/local/bin/claude \
 && rm -rf /root/.local
ENV DISABLE_AUTOUPDATER=1
COPY --from=daemon /app/src-tauri/target/release/guacad /usr/local/bin/guacad
COPY --from=web /app/dist /usr/share/guaca/web

# Every interface *inside the container*, which is the only way a published
# port reaches it; the container's network namespace is the boundary the
# loopback default draws on bare metal. Publish it to 127.0.0.1 on the host
# and put a tunnel in front, as the compose file does.
ENV GUACA_ROOT=/var/lib/guaca \
    GUACA_BIND=0.0.0.0:8787 \
    GUACA_WEB=/usr/share/guaca/web \
    GUAC_LOG=guac=info,warn
VOLUME /var/lib/guaca
EXPOSE 8787
USER guaca
# The one route without a token on it. Port spelled twice on purpose: a
# GUACA_BIND override that moves the port has to move this too, and a check
# that silently probed the wrong port would report a healthy box that is not.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s \
  CMD curl -fsS http://127.0.0.1:8787/health || exit 1
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/guacad"]
