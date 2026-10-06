# Plinth Registry Format — Specification

> Status: draft 1 (2026-10-06). This is a starting point; it can change. It defines the "source" of `docs/HUB.md` §5.1. It uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

---

## 1. Purpose

A **registry** is a place where the Plinth Hub and the `plinth` CLI find apps and download them. The design has these goals:

1. **One base URL is enough.** A client that knows the base URL can find everything else (as a NuGet v3 feed does with its service index).
2. **Static files are enough.** A folder of `.plnt` files plus some generated JSON files is a complete registry. Any static web server, a CDN, blob storage, or a local folder can serve it.
3. **A dynamic registry is compatible.** A registry web app can serve the same URLs from a database and blob storage, and it can add a search endpoint.
4. **Content addresses.** A package is stored and verified by its SHA-256 digest, so mirrors and caches cannot change it.
5. **Local development is simple:** put `.plnt` files in a folder, run `plinth registry build`, and serve the folder (or use the folder path directly).

---

## 2. Base URL and the service index

A registry has a **base URL**: an `https://` URL, an `http://` URL (allowed only for `localhost` and `127.0.0.1`), or a local folder path (`file:` URL or a plain path, for development).

The client reads the **service index** at:

```
<base>/plinth-registry.json
```

```json
{
  "schema": "plinth.registry/1",
  "name": "Example Apps",
  "description": "Apps from Example Corp.",
  "resources": {
    "apps": "apps/index.json",
    "app": "apps/{id}/index.json",
    "package": "packages/{sha256}.plnt",
    "cores": "cores/index.json",
    "core": "cores/{version}/core.wasm",
    "search": "search?q={query}"
  },
  "generated": "2026-10-06T12:00:00Z"
}
```

- `schema` is required. A client refuses a schema major version that it does not know.
- `resources` maps each resource to a URL template, **relative to the base URL** (or absolute). `{id}`, `{sha256}`, `{version}`, and `{query}` are placeholders; the client percent-encodes the values.
- `apps`, `app`, and `package` are required. `cores`, `core`, and `search` are optional.
- A static registry has no `search` resource. The client then searches the `apps` list itself.

---

## 3. The app list

`apps` gives a summary of every app in the registry:

```json
{
  "schema": "plinth.registry/1",
  "apps": [
    {
      "id": "com.example.notes",
      "name": "Notes",
      "summary": "Simple notes that stay on your device.",
      "publisher": "example",
      "latest": "1.2.0",
      "categories": ["productivity"],
      "icon": "com.example.notes/icon.png",
      "capabilities": ["store.kv"],
      "labels": [
        { "name": "store.kv", "risk": "low", "description": "save data on this device", "rationale": "Save your notes." }
      ],
      "updated": "2026-10-06T12:00:00Z"
    }
  ],
  "next": null
}
```

- `capabilities` are the capabilities of the latest version, so a list view can show the capability label without more requests.
- `labels` (optional) is the capability label of the latest version (`docs/HUB.md` §7.2): for each capability, the risk level (`none`, `low`, `medium`, `high`), the fixed description, and the reason of the app. `plinth registry build` writes it from the shared capability map (`crates/plinth-link/src/capabilities.rs`) each time it runs. `net:<host>` is medium ("connect to <host>"); `net:*` is high. A client must not trust it for consent decisions; the web App Hub (`docs/web-hub.md`) uses it only to show the label.
- `icon` (optional): `plinth registry build` copies the icon that the manifest names from the package to `apps/<id>/<path>`.
- URLs in the app list (for example `icon`) are relative to the app list's own URL, not to an app document.
- **Paging.** A large registry splits the list into pages. `next` is the URL of the next page, or `null`. A static registry can always use one page.
- App ids are compared in lower case.

---

## 4. The app document

`app` (`apps/{id}/index.json`) describes one app and all its versions:

```json
{
  "schema": "plinth.registry/1",
  "id": "com.example.notes",
  "name": "Notes",
  "summary": "Simple notes that stay on your device.",
  "description": "A longer text, plain or Markdown.",
  "publisher": "example",
  "homepage": "https://example.com/notes",
  "icon": "icon.png",
  "screenshots": ["1.2.0/screen0-compact.png", "1.2.0/screen0-wide.png"],
  "versions": [
    {
      "version": "1.2.0",
      "sha256": "9f2c…",
      "size": 5120,
      "core": "1.2",
      "ui-api": "1.3",
      "capabilities": [
        { "name": "store.kv", "rationale": "Save your notes on this device." }
      ],
      "reachable": ["store.kv"],
      "published": "2026-10-06T12:00:00Z",
      "yanked": null,
      "signer": "ed25519:9f2c…"
    }
  ]
}
```

- URLs in an app document are relative to the app document's own URL.
- `versions` is sorted from the newest to the oldest (semver order).
- `sha256` is the digest of the `.plnt` file. The client downloads `package` with this digest and checks it. A mismatch is an error.
- `core` is the core version that the app needs (`SPEC.md` §10.5); `ui-api` is the UI API version.
- `capabilities` come from the manifest; `reachable` comes from the analysis of the app module (`docs/HUB.md` §7.1). The generator computes both. A client must not trust them without a check: it computes them again from the package after the download.
- `yanked`: `null`, or a reason text. A yanked version is not installed for new users; an installed copy keeps working, and the Hub shows the reason (`docs/HUB.md` §6.4).
- `signer` (phase H1, `docs/HUB.md` §6.1): the publisher key id from the package's `signature.json`, omitted for an unsigned package. The generator (`plinth registry build`) verifies the signature before adding a version — a package whose signature does not check out is refused, not merely recorded as unsigned — and refuses a new version whose signer differs from the previous version's (key rotation is future work). A draft-1 registry with no `signer` field anywhere still works: an absent field means "unsigned", the same as before this field existed.

---

## 5. Packages

`package` (`packages/{sha256}.plnt`) serves the package file. The file never changes, so a server can cache it for ever. The client:

1. downloads the file,
2. checks the SHA-256,
3. reads the manifest and checks that `id` and `version` agree with the app document,
4. runs the normal load checks (`SPEC.md` §10.4, capabilities).

---

## 6. Cores

`cores` (`cores/index.json`) lists the runtime cores that the registry serves, so a Hub can install a core that an app needs:

```json
{
  "schema": "plinth.registry/1",
  "cores": [{ "version": "1.3", "sha256": "…", "size": 68608 }]
}
```

`core` (`cores/{version}/core.wasm`) serves the file. The client checks the digest, then the core's own version section (`SPEC.md` §10.5). Later, cores also need the Plinth project signature.

---

## 7. Search (optional, dynamic registries)

`search` returns the same shape as the app list (§3), filtered and ranked by the registry. Query parameters other than `q` are reserved.

---

## 8. Static registries

A static registry is a folder:

```
my-registry/
  plinth-registry.json
  apps/
    index.json
    com.example.notes/
      index.json
      icon.png
      1.2.0/screen0-compact.png
  packages/
    9f2c….plnt
  cores/
    index.json
    1.3/core.wasm
```

`plinth registry build <folder>` makes it. Its input is the `.plnt` files in the folder (or in `<folder>/incoming/`). For each package it:

1. reads and validates the package (the same checks as `plinth validate`),
2. copies it to `packages/<sha256>.plnt`,
3. updates `apps/<id>/index.json` (it adds the version; it never changes an existing version),
4. writes `apps/index.json` and `plinth-registry.json`,
5. optionally copies the built-in core to `cores/` (`--with-core`),
6. optionally renders screenshots with `plinth-shoot` (later).

Running it again is safe: the output for the same input is the same (stable order, stable JSON). For this, the generator does not use the clock for derived times: `published` is set one time when a version is added, and `generated` and `updated` are the latest `published` time of the versions that they cover.

`plinth registry serve <folder> [--port 8080]` serves the folder on `127.0.0.1` for local development. A client can also use the folder path directly as the base URL.

`plinth registry serve <folder> --web` also serves the web host files, on the same origin: the web App Hub runs the Hub app from the registry (`docs/web-hub.md`). `plinth registry build <folder> --hub-trusted-key <key id>` writes `hub.json` at the registry root: the Hub app id and the keys that the web App Hub trusts as Hub keys. `bash scripts/web-hub-demo.sh` makes and serves a registry of the example apps.

---

## 9. Clients

- `plinth hub source add <name> <base URL>`, `plinth hub source list`, `plinth hub source remove <name>`.
- `plinth hub search <text>` searches all sources.
- `plinth hub install <id>[@<version>] [--source <name>]` downloads, checks, and adds the app to the library. The library records the source of each app.
- `plinth hub update [<id>]` installs newer versions (with the consent rules of `docs/HUB.md` §7.3).
- `plinth hub pin <id> <version>` keeps an app on an installed version; `plinth hub pin <id> --latest` removes the pin. The Hub UI does the same with `plinth:hub` (`checkUpdates`, `update`, `pin`; `docs/HUB.md` §9.2).

The Hub caches the service index and the app documents and uses HTTP caching headers. It works offline with the cached data.

---

## 10. Security (draft 1)

- Packages are verified by digest. A registry cannot change a package without detection after a client saw its digest.
- Draft 1 had **no signatures**; phase H1 (`docs/HUB.md` §6.1) adds the optional `signer` field (§4) and package-level Ed25519 signatures (`signature.json` in the `.plnt`, `SPEC.md` §10.1). The client trusts the registry for the mapping from app id to digest; it does not yet trust the registry for the mapping from app id to publisher key — that needs a registry signature over the indexes and the transparency log (§6.3 of `docs/HUB.md`), still future work (H2).
- A signed package's signature is checked independently of the registry, by the client (`plinth_package::signature::verify`) and by the generator before it adds a version. A registry that serves a package whose signature does not check out is caught by this, not by trusting the registry.
- `http://` is allowed only for `localhost` and `127.0.0.1`.
- A client limits the size of each JSON document and each package.

---

## 11. Versioning of this format

`schema` is `plinth.registry/<major>`. New optional fields can appear in the same major version; clients ignore fields that they do not know. A change that old clients cannot read makes a new major version.
