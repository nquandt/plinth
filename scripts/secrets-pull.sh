#!/usr/bin/env bash
# Restore the Plinth keys and one device token from Azure Key Vault.
#   az login
#   bash scripts/secrets-pull.sh <device>      # device: mac | iphone | pc
# Env: PLINTH_KV (vault name), PLINTH_PUBLISHER_DIR, PLINTH_HUB_KEY_DIR,
#      PLINTH_ACCESS_DIR. Existing files are never overwritten.
set -euo pipefail
device="${1:?usage: scripts/secrets-pull.sh <mac|iphone>}"
kv="${PLINTH_KV:-plinthkv36610}"
pub="${PLINTH_PUBLISHER_DIR:-$HOME/.config/plinth/publisher}"
hub="${PLINTH_HUB_KEY_DIR:-$HOME/.plinth-hub-key}"
acc="${PLINTH_ACCESS_DIR:-$HOME/.plinth-access}"
get() { az keyvault secret show --vault-name "$kv" -n "$1" --query value -o tsv; }
put() { # name, path, decode?
  if [ -e "$2" ]; then echo "keep   $2 (exists)"; return; fi
  mkdir -p "$(dirname "$2")"; umask 077
  if [ "${3:-}" = b64 ]; then get "$1" | tr -d '\r' | base64 -d > "$2"; else get "$1" | tr -d '\r' > "$2"; fi
  echo "wrote  $2"
}
put plinth-publisher-key-b64 "$pub/key.ed25519" b64
put plinth-publisher-toml "$pub/publisher.toml"
put plinth-hub-key-b64 "$hub/key.ed25519" b64
put plinth-hub-toml "$hub/publisher.toml"
put "plinth-access-$device-env" "$acc/$device.env"
