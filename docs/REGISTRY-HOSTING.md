# Hosting your own registry

> Status: 2026-10-27. Parts 1 to 3 are built and live at `https://plinth.femur.dev/` (tunnel `plinth-registry`, started 2026-10-27). Parts 4 to 6 are plans: nothing in them is deployed. This document uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

The goal: the owner builds Plinth apps, publishes them to a personal registry, and installs them on a phone from the Hub. A registry is a folder of static files (`docs/REGISTRY.md` §8), so the first version needs no server code.

## 1. What exists

| Part | Where | State |
|---|---|---|
| Publish script | `scripts/registry-publish.sh` | Built. Builds and signs apps and the Hub app, runs `plinth registry build`, copies the web host files, and can copy the result to Azure (`--sync`). |
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

`plinth publish` (PRODUCTION T6) should send a signed package to a registry, so no folder copy is needed. Design:

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

Not built yet:

- The Hub client does not send the headers. A registry source needs a `headers` field (it is not in `docs/HUB.md` yet), and the secret must come from the keychain, not from the source list file.
- A second layer: a Cloudflare rate-limit rule for the name.

The Azure mirror (§1) has **no** Access. Its URL is public. Do not put private apps there, and do not use `--sync` for an app that must stay private.

## 6.1 Plan: a private registry (not done)

If some apps must stay private, use a second registry source with a different name and a different policy. Access already hides the whole registry from people without a login or a token.

## 7. Open items for the owner

1. Keep the PC on and Docker running. The registry is offline when they stop.
2. Decide when to do §4. The admin API token is in `~/.cloudflare/super.env`.
3. Decide if any app needs a private registry (§6.1).
4. Decide when to build the client `headers` support (§6).
