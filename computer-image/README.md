# The desktop image an agent's computer boots

`IMAGE_REF` is the one reference the app pulls, included verbatim at compile
time, so it holds a single line and no comment. The tag in it is a placeholder:
publishing under `ghcr.io/madebywelch/` is the maintainer's, and until it is
published nothing pulls. Set `GUAC_COMPUTER_IMAGE` to a locally built image to
try the feature meanwhile. The Dockerfile and the PID-1 watchdog land next.
