#!/usr/bin/env bash
# The web App Hub demo (docs/web-hub.md): builds the CLI and the example
# apps, writes a registry with the runtime core to target/web-hub-registry,
# and serves the registry and the web App Hub on http://127.0.0.1:8787/.
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

# Apps that do not make sense in this demo: `hub` is privileged (it needs
# hub.manage, which the browser host does not give), `gc-torture` and
# `big-list` are stress tests.
skip=" hub gc-torture big-list "

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

"$plinth" registry build "$registry" --with-core

if [ "$serve" = yes ]; then
  echo
  echo "Open http://localhost:$port/ in a browser."
  echo "For another device: cloudflared tunnel --url http://localhost:$port"
  echo "(the tunnel URL is public: anyone who has it can open the apps)"
  exec "$plinth" registry serve "$registry" --web --port "$port"
fi
