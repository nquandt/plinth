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

## 6. Plan: a private registry (not done)

If some apps must stay private:

1. Put Cloudflare Access in front of `plinth.femur.dev` with one allowed e-mail and a service token for the Hub.
2. The Hub client needs a header per source (`docs/HUB.md` §5.1: add `headers` to a source). This is not built.
3. The web App Hub works with an Access cookie with no client change.

## 7. Open items for the owner

1. Keep the PC on and Docker running. The registry is offline when they stop.
2. Decide when to do §4 and give a Cloudflare API token, if the owner wants the mirror to replace Docker.
3. Decide if any app needs a private registry (§6).
