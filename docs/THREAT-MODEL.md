# Threat model

This document tells what Plinth protects, from whom, and how. It is checklist item S10 in `docs/PRODUCTION.md`. Update it when a mitigation changes or a new host API comes in. `SECURITY.md` tells how to report a vulnerability.

Status of this file: first version, 2026-10-07.

## 1. Assets

| Asset | Where |
|---|---|
| The user's data in each app | Desktop: the per-app kv store and the private file space. Web: the page storage (kv) and IndexedDB (files), in one namespace for each app. |
| The user's other files and devices | The file system outside the app spaces; the local network (printers, routers, cloud metadata at `169.254.169.254`). |
| The user's consent decisions | The grants in the Hub and the host (`plinth-hub`). |
| The integrity of the code that runs | `.plnt` packages, runtime cores, the host binaries, update feeds. |
| Publisher identity | Publisher Ed25519 keys (`docs/HUB.md` §6.1); the project key (not decided yet). |
| The device | CPU, memory and the UI thread of the host. |

## 2. Actors

| Actor | Can do | Trust |
|---|---|---|
| **App** (its Wasm code) | Anything inside its own memory; calls to host imports. | None. An app can be written to attack. |
| **Publisher** | Ships packages and updates for its own app ids. | Trusted only for its own apps, and only with the same key. |
| **Registry / source** | Lists packages and digests. | Not trusted for content: the host checks digests and signatures. |
| **Network attacker** | Reads and changes traffic. | None. |
| **Web page that embeds the host** | Runs the parent of the app frames. | Trusted by the apps in it; the page origin owns the storage. |
| **The user** | Grants and refuses capabilities. | Trusted. Can make mistakes (consent fatigue). |
| **Plinth project** | Ships the host and the cores. | Trusted. The project key signs cores (planned). |

## 3. Trust boundaries

1. **App ↔ host (the Wasm sandbox).** The app is a Wasm module linked into a runtime core. It sees only its linear memory and the imports in `wit/plinth/app.wit`. Desktop: wasmtime (or wasmi). Web: the browser's Wasm engine, in a sandboxed iframe.
2. **App ↔ app.** Each app has its own instance, memory and data spaces. No host API names another app's data.
3. **Host ↔ network.** `plinth:net` requests go out only through the host, after the capability check.
4. **Host ↔ package source.** Packages, cores and update feeds come from outside and are checked before use.
5. **App frame ↔ parent page (web).** The frame is `sandbox="allow-scripts"` with an opaque origin; the only channel is `postMessage` (`docs/web-hub.md` §4.1).

## 4. Threats and mitigations

Status words as in `docs/PRODUCTION.md`: **Done**, **Partial**, **Open**.

### 4.1 App against the host and the device

| Threat | Mitigation | Status |
|---|---|---|
| Escape from the Wasm sandbox | wasmtime/wasmi and the browser enforce memory safety. The host links only the imports of `app.wit`. | Done (relies on the engines) |
| An import that the host does not know | The host proves the app's capabilities from its imports; an unknown import fails at link time. | Done |
| An endless loop or deep recursion freezes the host | Desktop: 2 s per call (epochs, fuel on wasmi), stack limit. Web: no limit yet. | Partial (web: R3) |
| An app allocates all memory | Desktop: 64 MiB cap (`Limits`). Web: no cap yet. | Partial (web: R3) |
| A malformed op stream or tree crashes the renderer | The UI tree checks each op; fuzz tests `hostile_ops.rs`, `hostile_render.rs`. | Done |
| A trap in a host call | Denied calls return an error value; they never trap (SPEC §8.5). | Done |

### 4.2 App against other apps and the user's data

| Threat | Mitigation | Status |
|---|---|---|
| Read another app's kv data | Desktop: one store for each app id. Web: the page sends the frame only the data of its own app id (`docs/web-hub.md` §4.2). | Done |
| Read files outside the app space (`..`, absolute paths, encoded forms) | The host derives every path (`docs/STORAGE.md` §3); path checks on both hosts. | Done |
| A second publisher uses the same app id | Storage and grants key on (publisher key, app id) (`docs/STORAGE.md` §3 rule 2). The Hub refuses a new version with a different key. | Partial (key rotation: S6) |
| Data exfiltration through `net` | The user sees the `net` capability on the consent screen. A granted `net:*` can send any data that the app has. | Accepted risk (§6) |

### 4.3 App against the network

| Threat | Mitigation | Status |
|---|---|---|
| Requests to hosts that the manifest does not declare | `net:<host>` or `net:*` must be declared and not refused. | Done |
| Requests to the local network, router or cloud metadata (SSRF) | A private host needs `net.local`. Desktop: `is_private_ip` covers all non-public IPv4 and IPv6 ranges and embedded IPv4, and a resolver drops private addresses after DNS resolution, so a public name that resolves to a private address (DNS rebinding) is also denied. Web: the same IP rules on the URL host (`isPrivateNetHost`). | Done on desktop; Partial on the web (§6) |
| A redirect to a host that is not allowed | The desktop runner follows no redirects. | Done |
| A very large response | Desktop: the body limit is 8 MiB (`MAX_BODY`). | Done |

### 4.4 Supply chain: packages, cores, updates

| Threat | Mitigation | Status |
|---|---|---|
| A changed package | Ed25519 signature over the digests of every entry; a package with a bad signature is refused, never treated as unsigned. | Done |
| A changed manifest in a signed package (a new `version` that defeats the no-downgrade check, the `id` of another app of the same publisher and thus its data, more capabilities such as `net:*`, another `[updates]` feed or `core` policy) | `plinth.signature/2` digests the raw bytes of `manifest.toml` and of every other entry (`plinth-profile`, `source/…`, unknown entries). Desktop and web check the bytes in the zip, never a new serialization. The old schema `/1`, which did not cover the manifest, is refused. Tests: `plinth-package` `signature::tests`, `web/test/run-hub-integrity.mjs`. | Done |
| An unsigned package | Accepted, with a warning; it gets an identity from its digest. | Accepted risk (§6) |
| A changed core | The host checks the core's SHA-256 digest against its list (`core-digest` in the manifest; the web Hub checks too). | Done |
| A malicious update | Signed feed (`feed.json.sig`), key continuity with the embedded package, no downgrade, digests, rollback after two failed starts (`docs/UPDATES.md`). | Done (desktop); Open (web, mobile) |
| An update that asks for new capabilities | The user must consent again. | Open (S5) |
| A source that shows different packages to different users | Transparency log (`docs/HUB.md` §6.3). | Open (S12) |
| A stolen publisher key | Revocation and key rotation (`docs/HUB.md` §6.4). | Open (S6) |
| A vulnerable dependency of the host | Dependency audit in CI. | Open (S8) |
| A changed host binary | Code signing and notarization. | Open (P5) |

### 4.5 Web host

| Threat | Mitigation | Status |
|---|---|---|
| An app frame reads the page, its storage or another frame | `sandbox="allow-scripts"`, opaque origin, CSP `sandbox allow-scripts` on `app-frame.html`. | Done |
| A frame sends a forged bridge message | The page checks each request against the capabilities of that frame's app before it acts. | Done |
| `hub.manage` from an app frame | Only the Hub app, signed by a trusted key; never an app frame. | Done |
| Code of the page reads the kv data of apps | The data is in the page origin's storage. The page is part of the host and is trusted. | Accepted risk (§6) |

## 5. Not in scope

- A malicious host binary, a compromised OS, or physical access to an unlocked device.
- Side channels between apps in one process (timing, Spectre). The engines give some protection; Plinth adds none.
- An app that does harm with the capabilities that the user granted. The consent screen is the control.

## 6. Accepted risks and open work

1. **Web `net` and DNS.** A web page cannot see where a name resolves, so on the web a public name that resolves to a private address is not caught by the host. Browsers with Private Network Access protection block some of these requests. Item S3 stays Partial on the web until a proxy or the browser covers it.
2. **Granted `net:*` can send data out.** The consent screen shows it as a high risk. A host policy can refuse `net:*` (`docs/HUB.md` §7.5).
3. **Unsigned packages run.** The Hub warns. A later policy can refuse them by default.
4. **Web storage is in the page origin.** Code of the host page can read it.
5. **No limits on the web host yet** (R3): an app can freeze its tab.
6. **Pinned cores.** An app with `core = "pinned"` keeps a core with a known defect until the author ships an update. The Hub must tell the user (planned).
7. **No third-party review yet** (S11).
