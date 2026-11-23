# syntax=docker/dockerfile:1.7

FROM debian:trixie-slim
ARG GUACA_COMMIT=""
ARG GUACA_VERSION=""
LABEL org.opencontainers.image.version=$GUACA_VERSION \
      org.opencontainers.image.revision=$GUACA_COMMIT
RUN apt-get update \
 && apt-get install -y --no-install-recommends chromium iptables socat ca-certificates fonts-liberation \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home-dir /tmp/chromium --shell /usr/sbin/nologin chromium \
 && useradd --system --home-dir /nonexistent --shell /usr/sbin/nologin relay \
 && chromium --version > /etc/chromium-version
COPY policy.json /etc/chromium/policies/managed/guaca.json
COPY --chmod=755 entrypoint.sh /usr/local/bin/guaca-browser
EXPOSE 9223
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/guaca-browser"]
