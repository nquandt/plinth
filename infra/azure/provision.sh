#!/usr/bin/env bash
# Makes the Azure resources for the Plinth registry mirror (idempotent).
#
#   bash infra/azure/provision.sh
#
# It makes: one resource group, one storage account with the static website
# feature on (a public, read-only copy of the registry folder), and prints
# the public URL. Cost: a few cents each month for a personal registry.
# Needs: `az login` and the subscription selected (`az account set`).
set -euo pipefail

RG="${PLINTH_AZ_RG:-plinth-rg}"
LOCATION="${PLINTH_AZ_LOCATION:-centralus}"
STATE_FILE="$(dirname "$0")/.account-name"   # git-ignored; keeps the generated name

if [ -s "$STATE_FILE" ]; then
  ACCOUNT="$(cat "$STATE_FILE")"
else
  ACCOUNT="plinthreg$(printf '%05d' $((RANDOM * RANDOM % 100000)))"
  echo "$ACCOUNT" > "$STATE_FILE"
fi

az group create --name "$RG" --location "$LOCATION" \
  --tags project=plinth purpose=registry-mirror -o none

if ! az storage account show -g "$RG" -n "$ACCOUNT" -o none 2>/dev/null; then
  az storage account create -g "$RG" -n "$ACCOUNT" -l "$LOCATION" \
    --sku Standard_LRS --kind StorageV2 --min-tls-version TLS1_2 \
    --allow-blob-public-access true --https-only true \
    --tags project=plinth -o none
fi

# Static website: serves the $web container with plain HTTP caching.
az storage blob service-properties update --account-name "$ACCOUNT" --auth-mode login \
  --static-website --index-document plinth-registry.json --404-document 404.html -o none

echo "account: $ACCOUNT"
az storage account show -g "$RG" -n "$ACCOUNT" --query primaryEndpoints.web -o tsv
