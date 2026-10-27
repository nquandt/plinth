#!/usr/bin/env bash
# Builds Plinth apps and publishes them to your personal registry.
#
#   bash scripts/registry-publish.sh <app dir>... [--sync]
#
# 1. `plinth build --sign --release` for each app (needs `plinth publisher init`).
# 2. Builds the Hub app (examples/hub) and signs it with a separate Hub key
#    (made one time in $PLINTH_HUB_KEY_DIR, publisher name "you").
# 3. `plinth registry build` makes the static registry in $PLINTH_REGISTRY_DIR,
#    with the runtime core and hub.json (the Hub key is the trusted Hub key).
# 4. Copies the web host files to $PLINTH_REGISTRY_DIR/web/, so a phone
#    browser can open the web App Hub (docs/web-hub.md) at the site root.
# 5. With --sync: copies the registry to the Azure static website
#    (infra/azure/provision.sh made it).
#
# The Docker registry (infra/registry) serves the same folder, so step 2 is
# enough for plinth.femur.dev to show the new version.
set -euo pipefail

PLINTH="${PLINTH:-plinth}"
REG="${PLINTH_REGISTRY_DIR:-$HOME/plinth-registry}"
sync=no
dirs=()
for a in "$@"; do
  if [ "$a" = "--sync" ]; then sync=yes; else dirs+=("$a"); fi
done
[ ${#dirs[@]} -gt 0 ] || { echo "usage: $0 <app dir>... [--sync]" >&2; exit 2; }

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HUBKEY="${PLINTH_HUB_KEY_DIR:-$HOME/.plinth-hub-key}"
mkdir -p "$REG/incoming"

# The Hub app, signed with the Hub key.
if [ -z "$(ls -A "$HUBKEY" 2>/dev/null)" ]; then
  PLINTH_PUBLISHER_DIR="$HUBKEY" "$PLINTH" publisher init --name you
fi
HUBID="$(PLINTH_PUBLISHER_DIR="$HUBKEY" "$PLINTH" publisher show | awk '{print $NF}')"
echo "== build hub"
PLINTH_PUBLISHER_DIR="$HUBKEY" "$PLINTH" build "$ROOT/examples/hub" --sign --release --out "$REG/incoming/hub.plnt"

for d in "${dirs[@]}"; do
  name="$(basename "$d")"
  echo "== build $name"
  "$PLINTH" build "$d" --sign --release --out "$REG/incoming/$name.plnt"
done

echo "== registry build $REG"
"$PLINTH" registry build "$REG" --with-core --hub-trusted-key "$HUBID"

echo "== web host files"
rm -rf "$REG/web"
mkdir -p "$REG/web/vendor/sqlite-wasm"
for f in "$ROOT"/web/*.html "$ROOT"/web/*.js "$ROOT"/web/*.css; do cp "$f" "$REG/web/"; done
cp "$ROOT"/web/vendor/sqlite-wasm/sqlite3.mjs "$ROOT"/web/vendor/sqlite-wasm/sqlite3.wasm "$REG/web/vendor/sqlite-wasm/"

if [ "$sync" = yes ]; then
  ACCOUNT="$(cat "$(dirname "$0")/../infra/azure/.account-name")"
  RG="${PLINTH_AZ_RG:-plinth-rg}"
  KEY="$(az storage account keys list -g "$RG" -n "$ACCOUNT" --query '[0].value' -o tsv)"
  up() { # <source> <dest-path> <cache-control>
    az storage blob upload-batch --account-name "$ACCOUNT" --account-key "$KEY" -d '$web' \
      -s "$1" --destination-path "$2" --overwrite --content-cache-control "$3" -o none
  }
  echo "== sync to Azure ($ACCOUNT)"
  [ -d "$REG/packages" ] && up "$REG/packages" packages "public, max-age=31536000, immutable"
  [ -d "$REG/cores" ]    && up "$REG/cores"    cores    "public, max-age=3600"
  [ -d "$REG/apps" ]     && up "$REG/apps"     apps     "no-cache"
  [ -d "$REG/web" ]      && up "$REG/web"      web      "no-cache"
  for f in plinth-registry.json hub.json; do
    [ -f "$REG/$f" ] && az storage blob upload --account-name "$ACCOUNT" --account-key "$KEY" -c '$web' \
      -f "$REG/$f" -n "$f" --overwrite --content-cache-control "no-cache" -o none
  done
  echo "registry mirror: $(az storage account show -g "$RG" -n "$ACCOUNT" --query primaryEndpoints.web -o tsv)"
fi
echo "done"
