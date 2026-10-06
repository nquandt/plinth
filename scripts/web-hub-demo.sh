#!/usr/bin/env bash
# The web App Hub demo (docs/web-hub.md): builds the CLI and the example
# apps, signs the Hub app (examples/hub) with a throwaway demo key in
# target/web-hub-demo-key (never the owner's key, never in the repository),
# writes a registry with the runtime core and hub.json (the demo key as the
# trusted Hub key) to target/web-hub-registry, and serves the registry and
# the web host on http://127.0.0.1:8787/.
#
#   bash scripts/web-hub-demo.sh              # build, then serve (Ctrl+C to stop)
#   bash scripts/web-hub-demo.sh --no-serve   # build only
#   PORT=9000 bash scripts/web-hub-demo.sh    # another port
#
# To open it from another device, run in a second terminal:
#   cloudflared tunnel --url http://localhost:8787
# CAUTION: anyone who has the tunnel URL can open the page and the apps.
#
# Run it from the repository root.
set -euo pipefail

port="${PORT:-8787}"
serve=yes
[ "${1:-}" = "--no-serve" ] && serve=no

# Stress tests that do not make sense in this demo.
skip=" gc-torture big-list "

cargo build -p plinth-cli
plinth=./target/debug/plinth
registry=target/web-hub-registry

# A new registry each time: the generator refuses a changed package with
# the same version, and a rebuilt example keeps its version.
rm -rf "$registry"
mkdir -p "$registry/incoming"

for dir in examples/*/; do
  name="$(basename "$dir")"
  [ -f "$dir/plinth.toml" ] || continue
  case "$skip" in *" $name "*) echo "skip $name"; continue ;; esac
  echo "build $name"
  "$plinth" build "$dir" > /dev/null
  for pkg in "$dir"dist/*.plnt; do
    cp "$pkg" "$registry/incoming/"
  done
done

# The demo Hub key: made one time, kept in target/ (git ignores it). The
# Hub manifest's publisher is "you", so the key has that name.
keydir=target/web-hub-demo-key
if [ -z "$(ls -A "$keydir" 2>/dev/null)" ]; then
  PLINTH_PUBLISHER_DIR="$keydir" "$plinth" publisher init --name you
fi
key="$(PLINTH_PUBLISHER_DIR="$keydir" "$plinth" publisher show | awk '{print $NF}')"
PLINTH_PUBLISHER_DIR="$keydir" "$plinth" sign "$registry/incoming/hub.plnt"

"$plinth" registry build "$registry" --with-core --hub-trusted-key "$key"

if [ "$serve" = yes ]; then
  echo
  echo "Open http://localhost:$port/ in a browser."
  echo "For another device: cloudflared tunnel --url http://localhost:$port"
  echo "(the tunnel URL is public: anyone who has it can open the apps)"
  exec "$plinth" registry serve "$registry" --web --port "$port"
fi
