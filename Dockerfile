# syntax=docker/dockerfile:1.7

ARG GUACA_COMMIT=""
ARG GUACA_RELEASE=0
ARG GUACA_VERSION=""

FROM node:22-bookworm-slim AS web
WORKDIR /app
RUN corepack enable
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY index.html menubar.html tsconfig.json vite.config.ts build-stamp.ts release-protocol.json ./
COPY src ./src
ARG GUACA_COMMIT
ARG GUACA_RELEASE
ARG GUACA_VERSION
ENV GUACA_COMMIT=$GUACA_COMMIT GUACA_RELEASE=$GUACA_RELEASE
RUN test "$GUACA_RELEASE" != 1 || test "$GUACA_VERSION" = "$(node -p 'require("./package.json").version')"
RUN pnpm build

FROM rust:1.99-bookworm AS daemon
WORKDIR /app/src-tauri
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./
COPY src-tauri/vendor ./vendor
RUN mkdir -p src/bin \
 && echo 'fn main() {}' > src/main.rs \
 && echo 'fn main() {}' > src/bin/guacad.rs \
 && echo 'fn main() {}' > src/bin/guaca-updater.rs \
 && echo '' > src/lib.rs \
 && echo 'fn main() {}' > build.rs \
 && cargo build --release --no-default-features --features server --bin guacad --bin guaca-updater \
 && rm -rf src build.rs target/release/deps/guac* target/release/deps/libguac*
COPY src-tauri/ ./
COPY release-protocol.json release-keys.pub main-keys.pub /app/
COPY deploy/browser/seccomp.json /app/deploy/browser/seccomp.json
ARG GUACA_COMMIT
ARG GUACA_RELEASE
ENV GUACA_COMMIT=$GUACA_COMMIT GUACA_RELEASE=$GUACA_RELEASE
RUN cargo build --release --no-default-features --features server --bin guacad --bin guaca-updater

FROM docker:28.5.2-cli AS docker

FROM node:22-bookworm-slim
ARG GUACA_COMMIT
ARG GUACA_RELEASE
ARG GUACA_VERSION
LABEL org.opencontainers.image.version=$GUACA_VERSION \
      org.opencontainers.image.revision=$GUACA_COMMIT \
      bot.guaca.release=$GUACA_RELEASE
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl python3 \
 && rm -rf /var/lib/apt/lists/* \
 && groupmod --new-name guaca node \
 && usermod --login guaca --home /var/lib/guaca node \
 && mkdir -p /var/lib/guaca \
 && chown guaca:guaca /var/lib/guaca
ARG CLAUDE_CODE_VERSION=2.1.260
RUN curl -fsSL https://claude.ai/install.sh -o /tmp/install-claude.sh \
 && bash /tmp/install-claude.sh "${CLAUDE_CODE_VERSION}" \
 && install -m 755 /root/.local/bin/claude /usr/local/bin/claude \
 && rm -rf /root/.local /tmp/install-claude.sh
ENV DISABLE_AUTOUPDATER=1
COPY --from=daemon /app/src-tauri/target/release/guacad /usr/local/bin/guacad
COPY --from=daemon /app/src-tauri/target/release/guaca-updater /usr/local/bin/guaca-updater
COPY --from=docker /usr/local/bin/docker /usr/local/bin/docker
COPY --from=web /app/dist /usr/share/guaca/web

ENV HOME=/var/lib/guaca \
    GUACA_ROOT=/var/lib/guaca \
    GUACA_BIND=0.0.0.0:8787 \
    GUACA_WEB=/usr/share/guaca/web \
    GUAC_LOG=guac=info,warn
VOLUME /var/lib/guaca
EXPOSE 8787
USER guaca
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s \
  CMD curl -fsS http://127.0.0.1:8787/health || exit 1
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/guacad"]
