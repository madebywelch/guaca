#!/bin/sh
set -eu

CDP_PORT=9222
RELAY_PORT=9223
BROWSER=chromium
RELAY=relay

if iptables -w -L OUTPUT >/dev/null 2>&1; then
  IPT=iptables
  IPT6=ip6tables
else
  IPT=iptables-legacy
  IPT6=ip6tables-legacy
fi

for user in "$BROWSER" "$RELAY"; do
  "$IPT" -w -A OUTPUT -m owner --uid-owner "$user" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
done
"$IPT" -w -A OUTPUT -m owner --uid-owner "$RELAY" -o lo -d 127.0.0.1 -p tcp --dport "$CDP_PORT" -j ACCEPT
"$IPT" -w -A OUTPUT -m owner --uid-owner "$RELAY" -j REJECT

"$IPT" -w -A OUTPUT -m owner --uid-owner "$BROWSER" -d 127.0.0.11 -j ACCEPT
"$IPT" -w -A OUTPUT -m owner --uid-owner "$BROWSER" -p udp -j REJECT
for net in 0.0.0.0/8 10.0.0.0/8 100.64.0.0/10 127.0.0.0/8 169.254.0.0/16 172.16.0.0/12 \
  192.0.0.0/24 192.0.2.0/24 192.168.0.0/16 198.18.0.0/15 198.51.100.0/24 203.0.113.0/24 \
  224.0.0.0/4 240.0.0.0/4; do
  "$IPT" -w -A OUTPUT -m owner --uid-owner "$BROWSER" -d "$net" -j REJECT
done
dotted() {
  printf '%d.%d.%d.%d' "0x$(echo "$1" | cut -c7-8)" "0x$(echo "$1" | cut -c5-6)" \
    "0x$(echo "$1" | cut -c3-4)" "0x$(echo "$1" | cut -c1-2)"
}
tail -n +2 /proc/net/route | while read -r _ dest gateway _ _ _ _ mask _; do
  if [ "$dest" != 00000000 ]; then
    "$IPT" -w -A OUTPUT -m owner --uid-owner "$BROWSER" -d "$(dotted "$dest")/$(dotted "$mask")" -j REJECT
  fi
  if [ "$gateway" != 00000000 ]; then
    "$IPT" -w -A OUTPUT -m owner --uid-owner "$BROWSER" -d "$(dotted "$gateway")" -j REJECT
  fi
done

if [ -s /proc/net/if_inet6 ]; then
  "$IPT6" -w -A OUTPUT -m owner --uid-owner "$BROWSER" -j REJECT
  "$IPT6" -w -A OUTPUT -m owner --uid-owner "$RELAY" -j REJECT
fi

install -d -m 700 -o "$BROWSER" -g "$BROWSER" /tmp/chromium

setpriv --reuid="$RELAY" --regid="$RELAY" --init-groups --inh-caps=-all --bounding-set=-all --no-new-privs \
  socat "TCP-LISTEN:${RELAY_PORT},fork,reuseaddr" "TCP:127.0.0.1:${CDP_PORT}" &

exec setpriv --reuid="$BROWSER" --regid="$BROWSER" --init-groups --inh-caps=-all --bounding-set=-all --no-new-privs \
  env HOME=/tmp/chromium chromium \
  --headless=new \
  --remote-debugging-address=127.0.0.1 \
  --remote-debugging-port="$CDP_PORT" \
  --user-data-dir=/tmp/chromium/profile \
  --no-first-run \
  --no-default-browser-check \
  --disable-dev-shm-usage \
  --disable-background-networking \
  --disable-component-update \
  --disable-sync \
  --disable-quic \
  --force-webrtc-ip-handling-policy=disable_non_proxied_udp \
  --password-store=basic \
  about:blank
