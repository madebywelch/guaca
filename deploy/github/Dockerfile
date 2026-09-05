FROM python:3.12-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends openssl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY github_app.py .
# This service executes no repository code. Only it mounts the App private key.
# Root can read Docker's private-key secret and initialize a UID 1000 client token.
ENTRYPOINT ["python3", "/app/github_app.py", "serve", "/run/secrets/github_config"]
HEALTHCHECK --interval=10s --timeout=3s CMD python3 -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8791/health', timeout=2)"
