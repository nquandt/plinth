# The web App Hub

> Status: draft 2 (2026-10-06). This document uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

The web App Hub runs the Plinth Hub in a browser. There is one Hub UI: the Plinth app `examples/hub` (`docs/HUB.md` §4.1). The web host runs the same `.plnt` file that the desktop host runs. The browser page only starts it and does the host work: the `plinth:hub` backend, the consent window, and the app window.

A static server gives the registry (`docs/REGISTRY.md`) and the web host files on one origin. Use the web App Hub to test apps in a browser on a laptop or a phone.

## 1. Start the demo

Run this command from the repository root:

```sh
bash scripts/web-hub-demo.sh
```

The script does these steps:

1. It builds the `plinth` CLI and the example apps, also `examples/hub`. It does not build `gc-torture` and `big-list` (stress tests).
2. It makes a throwaway demo publisher key in `target/web-hub-demo-key` (one time), with the name `you` (the publisher of `examples/hub`). It signs the Hub package with this key. The key is not the key of the owner, and it is not in the repository (`target/` is ignored).
3. It writes a new registry with the runtime core to `target/web-hub-registry`: `plinth registry build --with-core --hub-trusted-key <demo key id>`. The build writes `hub.json` (§5).
4. It starts the server on `http://127.0.0.1:8787/` (`plinth registry serve target/web-hub-registry --web --port 8787`).

Open `http://localhost:8787/` in a browser. To stop the server, push `Ctrl+C`.

Options:

- `bash scripts/web-hub-demo.sh --no-serve` builds the registry and does not start the server.
- `PORT=9000 bash scripts/web-hub-demo.sh` uses a different port.

To serve your own registry folder:

```sh
plinth sign <folder>/hub.plnt                       # with your Hub key
plinth registry build <folder> --with-core --hub-trusted-key <your Hub key id>
plinth registry serve <folder> --web [--port N]
```

## 2. Open the Hub from a different device

Use a Cloudflare quick tunnel. `cloudflared` must be installed. Start the demo, then run this command in a second terminal:

```sh
cloudflared tunnel --url http://127.0.0.1:8787
```

`cloudflared` writes a URL such as `https://<random-words>.trycloudflare.com`. Open this URL on the laptop or the phone. To stop the tunnel, push `Ctrl+C`.

> **CAUTION:** A quick tunnel URL is public. Anyone who has the URL can open the page and run the apps until you stop the tunnel. Do not put private apps or private data in the registry. Stop the tunnel when you do not use it.

The page uses HTTPS through the tunnel. A browser gives the clipboard and WebCrypto only to a secure page (HTTPS or `localhost`).

## 3. How the page starts the Hub

`/` sends the browser to `/web/hub.html`. `hub.html` is a thin bootstrap (`hub-shell.js`):

1. It reads the registry at the root of the origin (`?registry=<url>` reads a different one) and `hub.json` (§5).
2. It downloads the latest version of the Hub app (`hub.json` `hub`, normally `dev.plinth.hub`) and a core that can run it (`SPEC.md` §10.5).
3. It checks the package and the core (§4.3).
4. It gives `hub.manage` to the Hub app only if the signature is correct and a key in `hub.json` `trustedKeys` made it (`docs/HUB.md` §4.1). If not, the page shows the reason and does not run the Hub. A browser that cannot check Ed25519 cannot trust the Hub app.
5. It runs the Hub app in the page with the `plinth:hub` backend (`hub-host.js`).

### 3.1 Browse mode: the library is the registry

On the web, the Hub does not install apps (`SPEC.md` §18, browse mode). The library is the app list of the registry, without the Hub app itself:

- `listApps` and `appInfo` give each registry app, with the same JSON fields as the desktop `HubService` (`crates/plinth-hub`, `wit/plinth/app.wit`).
- The browser keeps only the state of the user, in IndexedDB (`plinth-web-hub`): grants, blocks, publisher blocks, pins, groups, the apps that the user removed, and for each app the version that the user accepted.
- `install` shows an app again that the user removed. `remove` hides the app in this browser. Its data and grants stay.
- `search` searches the app list of the registry (the same rule as `plinth hub search` for a static registry).
- `checkUpdates` reads the registry again. If the registry has a newer version than the accepted one, the Hub shows "Update available" and the capabilities that the new version adds. `update` accepts the newer version.
- `pin` keeps an app on an accepted version or an older one.

The page downloads a package each time that the app opens and checks it (§4.3). The browser HTTP cache can keep it.

## 4. Running an app

**Open** in the Hub app calls `launch`. The page then does what the desktop host does:

1. A blocked app (or an app of a blocked publisher) does not open. The page shows a message.
2. **The consent window.** If the version has capabilities that are not decided, the page shows the consent window first. The window is part of the host, not of the Hub app. It shows the description, the risk level and the reason of the app for each capability, with **Allow** and **Deny**. Deny is selected first. None and Low risk capabilities are allowed by default and the window does not ask for them (`docs/HUB.md` §7.2). **Cancel** runs the newest accepted version whose capabilities are all decided, if there is one (`docs/HUB.md` §7.3 step 3).
3. **The app window.** The page downloads and checks the package, and shows the app in a window over the Hub (the Hub is inert while the window is open). **Close** or `Escape` in the window bar closes it.

The decisions go to the Hub state. The Hub app shows them on the app page (after **Refresh**). The user can change them there with the toggle of each capability.

### 4.1 Isolation

Each app runs in `<iframe sandbox="allow-scripts" src="app-frame.html">`. The frame has an opaque origin. It cannot read the storage, the cookies, IndexedDB or the DOM of the page or of a different app. The server also sends `Content-Security-Policy: sandbox allow-scripts` for `app-frame.html`, so the page has an opaque origin also if a browser opens it directly.

The frame and the page talk only through `postMessage` (the bridge, `web/README.md`). The frame side (`app-frame.js`) runs the core, the app and the DOM renderer. The page side (`frame-host.js`) does the host work for the frame. Every page that shows a Plinth app uses the same pair; the Hub page adds only the Hub parts.

### 4.2 Storage of an app

`plinth:store` calls are synchronous in the app. Thus:

1. The page keeps the kv data of each app in its own namespace of the page storage (`localStorage`, keys `plinth-hub:kv:<app id>:<key>`).
2. At the start, the page sends the kv data of this app only (a snapshot) and the quota to the frame.
3. The frame answers each `get` and `keys` from its copy, and sends each `set` and `delete` to the page.
4. The quota is 512 Ki characters of keys and values for each app. The frame and the page apply the same quota. A `set` over the quota answers `denied(refused)`.

With `store.kv` denied, the frame gets no data and each call answers `denied(refused)`.

### 4.3 Package checks

Before the page runs a package (the Hub app and each app), it checks:

1. The SHA-256 digest of the file against the digest in the registry (WebCrypto `crypto.subtle.digest`). A mismatch stops the app ("does not match the registry digest").
2. The manifest: the app id, the version, and the digest of the app module.
3. The signature, if the package has `signature.json` (`docs/HUB.md` §6.1): the digest of each entry, the publisher, and the Ed25519 signature (WebCrypto Ed25519). The signer must be the `signer` of the registry. If the browser has no Ed25519, the result is "not checked": an app still runs (the digest protects it), but the Hub app does not get `hub.manage`.
4. The core: its SHA-256 digest against the core list.

### 4.4 Capabilities in the browser

| Capability | In the browser |
|---|---|
| `store.kv` | Works. The page keeps the data (§4.2). |
| `clipboard.write` | Works. The page writes to the clipboard for the frame. |
| `clipboard.read` | Limited. The page reads the clipboard for the frame; the browser asks for permission. The first read can return nothing (the browser API is asynchronous; the host keeps the last value). |
| `net:<host>`, `net:*`, `net.local` | Limited. The page makes the request for the frame (after the same capability check), so the request has the origin of the page, not `null`. The server must allow requests from this origin (CORS). The request has no cookies. |
| `hub.manage` | Only the Hub app, signed by a trusted key (§3). An app that runs in a frame never gets it. |
| Other names | Not supported. Calls are refused. |

A sandboxed frame without `allow-modals` cannot show `alert`, `confirm` or `prompt`. The page draws these dialogs for `plinth:dialog`.

## 5. `hub.json`

`plinth registry build <folder> --hub-trusted-key <key id>` (the option can be given more than one time, or with comma-separated ids) writes `hub.json` at the root of the registry:

```json
{
  "schema": "plinth.web-hub/1",
  "hub": "dev.plinth.hub",
  "trustedKeys": ["ed25519:…"]
}
```

Without the option, the build does not write or change `hub.json`. It is a static file of the registry: the server has no Hub logic.

> **NOTE:** The page and `hub.json` come from the same server. Thus the trust in the Hub key is the trust in the server. A Plinth project Hub key in the page is future work (`HANDOFF.md`).

## 6. The server

`plinth registry serve <folder> --web` is a static file server:

| Path | What |
|---|---|
| `/` | A redirect to `/web/hub.html`. |
| `/web/…` | The web host files. They are in the `plinth` binary (`crates/plinth-registry/src/web_files.rs`), so the server works from any folder. Each one has `Access-Control-Allow-Origin: *`: a module script of the sandboxed frame is a CORS request with `Origin: null`. `app-frame.html` also has `Content-Security-Policy: sandbox allow-scripts`. |
| Other paths | The files of the registry folder, also `hub.json`. |

- The server binds to `127.0.0.1` only. Another device can connect only through a tunnel.
- Each response has `Cache-Control: no-cache`, so the browser gets a rebuilt app or registry at once.
- `.wasm` files have the type `application/wasm`.
- The server refuses paths outside the folder (`..`, absolute paths, links to other folders).
- Without `--web`, the server serves only the registry folder (`docs/REGISTRY.md` §8).

## 7. Limits (draft 2)

- All app data is in the storage of one origin, in a namespace for each app. The apps cannot read it (they run in opaque origins), but code of the page can. Other pages of the same origin (for example `index.html`, the stand-alone host for tests and development) can read it too.
- The consent window comes before the first run. There are no first-use prompts yet (`docs/HUB.md` §7.3 step 2, H5).
- The Hub app does not get a notice when the host changes a grant (after the consent window). **Refresh** shows the change.
- Decisions for High risk capabilities last until the user changes them, as on the desktop. `docs/HUB.md` §7.3 step 4 asks for session grants in browse mode; that is future work.
- One app window at a time.
- The registry has no `summary` or `description` from the manifest yet, so most apps show no summary.

## 8. Tests

- `cargo test -p plinth-registry --test web_hub`: the redirect, the types, `no-cache`, CORS and the sandbox header, the embedded files, the path checks, and `hub.json` from `plinth registry build`.
- `node web/test/run-hub-host.mjs`: the `plinth:hub` backend against a served registry (browse mode, consent, grants, groups, pins, blocks, updates with a new capability, the digest check), and the Hub app itself on the web host with no DOM.
- `node web/test/run-hub-storage.mjs`: the bridge messages, the kv namespaces, the snapshot and the writes, the quota, the `net` check, and notes through the bridge.
- `node web/test/run-hub-integrity.mjs`: digests, manifest checks, Ed25519 signatures (made with `plinth sign`), the "not checked" result, and the core digest.
- `node web/test/run-a11y.mjs` (by hand; it needs Edge and the axe-core CDN): the Hub app from a registry in headless Edge (light, dark, phone width), Discover, apps in the sandboxed frame (the frame cannot read storage, cookies or the page), the consent window, kv isolation between notes and budget, a grant that the Hub app turns off, a package with a wrong digest, typing in todo and the stopwatch in the frame. axe-core finds no serious problems in the Hub views, the consent window, the app window and the app in the frame. `--hub-only` runs only the Hub part. `A11Y_SHOTS=<folder>` also saves screenshots.
