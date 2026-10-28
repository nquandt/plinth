# Hosting your own registry

> Status: 2026-10-27. Parts 1 to 3 are built and live at `https://plinth.femur.dev/` (tunnel `plinth-registry`, started 2026-10-27). Parts 4 to 6 are plans: nothing in them is deployed. This document uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

The goal: the owner builds Plinth apps, publishes them to a personal registry, and installs them on a phone from the Hub. A registry is a folder of static files (`docs/REGISTRY.md` §8), so the first version needs no server code.

## 1. What exists

| Part | Where | State |
|---|---|---|
| Publish script | `scripts/registry-publish.sh` | Built. Calls `plinth publish` (§2.1) for the apps, builds and signs the Hub app, runs `plinth registry build`, copies the web host files, and can copy the result to Azure (`--sync`). |
| Docker site and tunnel | `infra/registry/` | Built. Caddy serves the folder; `cloudflared` publishes it at `plinth.femur.dev`. Steps: `infra/registry/README.md`. |
| Azure mirror | `infra/azure/provision.sh` | Built and run on 2026-10-27. |
| Web App Hub | `docs/web-hub.md` | Built. The site root opens it. |

Azure resources (subscription "Pay-As-You-Go"):

| Name | What |
|---|---|
| Resource group `plinth-rg` (`centralus`) | Holds everything for Plinth. Delete the group to remove it all. |
| Storage account `plinthreg36610` (Standard LRS, StorageV2) | The static website feature is on. The `$web` container holds a public copy of the registry folder. |
| Mirror URL | `https://plinthreg36610.z19.web.core.windows.net/` |

The mirror has the same files as the Docker site. It is a backup, and it is the second way to serve the registry when the owner's PC is off.

## 2. The publish flow

```text
my-app/ ──plinth build --sign --release──▶ ~/plinth-registry/incoming/my-app.plnt
                                              │ plinth registry build --with-core
                                              ▼
                              ~/plinth-registry/{plinth-registry.json, apps/, packages/, cores/, hub.json}
                                   │                                   │
                  Docker (Caddy + cloudflared)                  --sync (az blob upload)
                                   ▼                                   ▼
                       https://plinth.femur.dev/          https://plinthreg36610.z19.web.core.windows.net/
```

### 2.1 `plinth publish`

One command does the first step and the registry build for one app:

```bash
plinth publish [<app dir>] [--registry <folder>] [--with-core] [--sync] [--check]
```

1. It reads `plinth.toml`. If `publisher` is not the name of the signing key, it stops with a message and writes nothing.
2. It builds the app with `--sign --release` into a scratch folder, and runs the registry build there with a copy of the app document of your registry. All checks of `plinth registry build` run, and the registry is not changed. If the registry would refuse the package, the command stops here. (A different package with the same app id and version is refused. The same package again is "unchanged". A key that differs from the key of the previous version is refused.)
3. It puts the package in `<registry>/incoming/<folder name>.plnt` and runs `plinth registry build` on the registry folder. `plinth-registry.json`, `apps/` and `packages/` change. With `--with-core` it also copies the built-in core to `cores/`.
4. With `--sync`, it runs `sync_command` (§2.2).

`--check` is a dry run: it builds, validates, prints what it would add or keep, and does not write to the registry. It cannot be used with `--sync`.

The registry folder is, in this order: `--registry`, the variable `PLINTH_REGISTRY_DIR`, `registry_dir` in the config file, `~/plinth-registry`.

### 2.2 The config file and `--sync`

The config file is `%APPDATA%\plinth\publish.toml` on Windows and `$XDG_CONFIG_HOME/plinth/publish.toml` (or `~/.config/plinth/publish.toml`) elsewhere. `PLINTH_PUBLISH_CONFIG` sets another path. All keys are optional:

```toml
registry_dir = "~/plinth-registry"
registry_url = "https://plinth.femur.dev"   # printed as an install hint
sync_command = "bash /path/to/plinth/scripts/registry-publish.sh --sync"
```

`--sync` runs `sync_command` in a shell, in the current folder, after the publish, and shows the command before it runs. It sets `PLINTH_REGISTRY_DIR` to the registry folder. There is no default command: with `--sync` and no `sync_command`, `plinth publish` stops before it changes anything. The command above is the Azure example (§1). Plinth does not hardcode a host. Any command that uploads the folder works, for example `rsync -a --delete "$PLINTH_REGISTRY_DIR"/ host:/srv/registry/`.

`scripts/registry-publish.sh` calls `plinth publish` for each app, and keeps the Hub app, the web host files and the Azure upload to itself. Run with `--sync` and no app folder, it rebuilds the Hub app and uploads.

Tested on Windows: `crates/plinth-cli/tests/publish.rs` (two versions, a repeat, a refused change, the publisher rule, `--check`, the config and `--sync` with a command that writes a file), and the script with a scratch copy of `examples/counter` and a temporary registry folder. No test uploads to Azure.

- A package is named by its SHA-256 digest and never changes. The index files change on each publish.
- Two keys exist. The **publisher key** signs apps (its name must equal `publisher` in `plinth.toml`). The **Hub key** signs only the Hub app, and its id is the trusted key in `hub.json` (`docs/web-hub.md` §5). Keep both keys on the owner's machine. Never put a key in the registry folder or in Docker.
- To install on a phone: add the registry URL as a source in the Hub, then install the app (`docs/HUB.md`).

## 3. Why this design

- The registry is static, so the cheapest host works and a failure is easy to understand.
- Docker plus a named tunnel needs no open port and no certificate work. The cost is that the PC must be on.
- The Azure mirror costs a few cents each month. It makes a later move easy: point the name at the mirror and stop Docker (§4).
- Data of the apps never goes to the registry. Only app code does (`docs/STORAGE.md`).

## 4. Plan: serve the mirror at plinth.femur.dev (not done)

This needs a Cloudflare API token with `Zone.DNS:Edit` and `Zone.Rules:Edit` for `femur.dev`, or the owner's help in the dashboard.

1. Add a proxied `CNAME` record `plinth` to `plinthreg36610.z19.web.core.windows.net`.
2. Add an origin rule that sets the `Host` header to the same Azure name. (Azure static websites answer only for their own host name.)
3. Set SSL mode "Full (strict)". Cache rules: `/packages/*` and `/cores/*` for one year; JSON files "bypass cache".
4. Stop the tunnel and delete the tunnel DNS record.

Also possible: Cloudflare R2 with a custom domain, which removes the Host header problem. Azure is the choice for now because the owner has credit there and the account exists.

## 5. Plan: a registry that accepts uploads (not done)

Today `plinth publish` (§2.1) writes to a registry folder. A later step should send a signed package to a registry, so no folder copy is needed. Design:

1. A small service on Azure Container Apps (consumption plan, scale to zero) or a Cloudflare Worker. It runs `plinth-registry` code (library only; it does not need `gpui`, so a Linux image builds fast).
2. The upload is the signed `.plnt`. The service checks the package signature and compares the signer with the allowed keys of the registry (`allowed-publishers.json`). No password and no token for the publisher: the signature is the proof.
3. The service writes the package and the new indexes to Blob Storage, with an ETag check so two uploads cannot overwrite each other.
4. Rate limits and a size limit protect the service (`docs/LIMITS.md`).

Do not build it before the static flow has worked for a month. A script is enough for one person.

## 6. Access control (built, 2026-10-27)

Cloudflare Access protects `plinth.femur.dev`. A request without a login or a token gets a redirect (302) at the Cloudflare edge and never reaches the PC. The Access application "Plinth registry" (account of the owner) has two policies:

| Policy | Who | How |
|---|---|---|
| Operator only (allow) | The owner's e-mail | Browsers log in once and keep a cookie (24 h). The web Hub and the iOS web shell need no client change. |
| Plinth devices (service auth) | Service tokens `plinth-pc`, `plinth-mac`, `plinth-iphone` | A native host sends the headers `CF-Access-Client-Id` and `CF-Access-Client-Secret`. |

Rules for the tokens:

- Do not build a token into a binary. A binary can be read. Each device keeps its own token: the owner enters it once, and the host stores it in the OS keychain.
- The secrets are in `~/.plinth-access/<device>.env` on the PC. They are not in git. Cloudflare shows a secret only once, so a lost secret needs a new token.
- Each token expires after one year (2027-10-27). Make a new token, add it to the policy "Plinth devices", and delete the old one.
- To revoke one device: delete its token (Zero Trust, Access, Service Auth). The other devices keep working.
- The admin API token (`~/.cloudflare/super.env`) is for one-off set-up only. Never use it in an app or in CI.

Test commands (replace the file name for the device):

```bash
set -a; . ~/.plinth-access/mac.env; set +a
curl -s -o /dev/null -w "%{http_code}
" -H "CF-Access-Client-Id: $CF_ACCESS_CLIENT_ID" -H "CF-Access-Client-Secret: $CF_ACCESS_CLIENT_SECRET" https://plinth.femur.dev/plinth-registry.json
```

Without the headers the answer is 302. With a valid token it is 200.

The native client sends the headers (built, 2026-10-27). A source can carry request headers, and the secret is in the system keychain, not in the sources file (`docs/HUB.md` §5.4). To use a device token:

```bash
set -a; . ~/.plinth-access/mac.env; set +a
plinth hub source add femur https://plinth.femur.dev
plinth hub source set-token femur      # reads CF_ACCESS_CLIENT_ID and CF_ACCESS_CLIENT_SECRET
plinth hub search notes
```

The client sends the headers only to the host of the source. A redirect to another host does not get them. A redirect to a Cloudflare Access login page gives the error "the registry needs a login or a token (Cloudflare Access)". The web Hub needs no change: the browser keeps the Access cookie.

Tested: a local test server that answers 302 unless both headers are present (`crates/plinth-registry/tests/source_headers.rs`), and the keychain on Windows. Not tested against the real `plinth.femur.dev`, and not on macOS or Linux.

Not built yet:

- The mobile hosts (iOS, Android) have no field for the token.
- A second layer: a Cloudflare rate-limit rule for the name.

The Azure mirror (§1) has **no** Access. Its URL is public. Do not put private apps there, and do not use `--sync` for an app that must stay private.

## 6.1 Plan: a private registry (not done)

If some apps must stay private, use a second registry source with a different name and a different policy. Access already hides the whole registry from people without a login or a token.

## 6.2 Secrets in Azure Key Vault (built, 2026-10-27)

The keys and the device tokens are in the Key Vault `plinthkv36610` (resource group `plinth-rg`, `centralus`, RBAC mode, 90-day soft delete, the owner has "Key Vault Secrets Officer").

| Secret | Content |
|---|---|
| `plinth-publisher-key-b64`, `plinth-publisher-toml` | The publisher key "Nate" (`key.ed25519` in base64, and `publisher.toml`). |
| `plinth-hub-key-b64`, `plinth-hub-toml` | The Hub key "you". |
| `plinth-access-mac-env`, `plinth-access-iphone-env` | The Access service tokens for those devices. |

Not in the vault: the tunnel token (`infra/registry/.env`, only the PC needs it) and the admin API token (`~/.cloudflare/super.env`).

On the Mac, run `az login`, then `bash scripts/secrets-pull.sh mac`. The script writes the files to the paths that `plinth` and the publish script read. It never overwrites a file. A round trip on the PC gave the same bytes.

Cautions:

- The vault holds the private keys. Anyone with "Key Vault Secrets User" on it can read them. Give that role to nobody else.
- After a key change, put the new secret in the vault (`az keyvault secret set`). A new version replaces the old one for `show`.

## 7. Open items for the owner

1. Keep the PC on and Docker running. The registry is offline when they stop.
2. Decide when to do §4. The admin API token is in `~/.cloudflare/super.env`.
3. Decide if any app needs a private registry (§6.1).
4. Decide when to build the client `headers` support (§6).
