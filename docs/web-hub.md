# The web App Hub

> Status: draft 1 (2026-10-06). This document uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

The web App Hub is a page that lists the apps of a registry (`docs/REGISTRY.md`) and runs them in the web host (`web/`). One local server gives the registry and the page on one origin. Use it to test apps in a browser on a laptop or a phone.

## 1. Start the demo

Run this command from the repository root:

```sh
bash scripts/web-hub-demo.sh
```

The script does these steps:

1. It builds the `plinth` CLI.
2. It builds the example apps. It does not build `hub` (it needs `hub.manage`, which the browser host does not give), `gc-torture` and `big-list` (stress tests).
3. It writes a new registry with the runtime core to `target/web-hub-registry` (`plinth registry build --with-core`).
4. It starts the server on `http://127.0.0.1:8787/` (`plinth registry serve target/web-hub-registry --web --port 8787`).

Open `http://localhost:8787/` in a browser. To stop the server, push `Ctrl+C`.

Options:

- `bash scripts/web-hub-demo.sh --no-serve` builds the registry and does not start the server.
- `PORT=9000 bash scripts/web-hub-demo.sh` uses a different port.

To serve your own registry folder:

```sh
plinth registry build <folder> --with-core
plinth registry serve <folder> --web [--port N]
```

## 2. Open the Hub from a different device

Use a Cloudflare quick tunnel. `cloudflared` must be installed. Start the demo, then run this command in a second terminal:

```sh
cloudflared tunnel --url http://127.0.0.1:8787
```

`cloudflared` writes a URL such as `https://<random-words>.trycloudflare.com`. Open this URL on the laptop or the phone. To stop the tunnel, push `Ctrl+C`.

> **CAUTION:** A quick tunnel URL is public. Anyone who has the URL can open the page and run the apps until you stop the tunnel. Do not put private apps or private data in the registry. Stop the tunnel when you do not use it.

The page uses HTTPS through the tunnel. Thus the browser gives the clipboard to the apps (a browser gives the clipboard only to a secure page). On `http://localhost`, the clipboard also works.

## 3. What the page shows

- **The app list:** the name, the publisher, the version, the summary, the icon (if the package has one), and the capabilities of each app. A search box filters the list by name, id, publisher, summary or capability.
- **The app page:** the capability label (`docs/HUB.md` §7.2). Each capability has its description, its risk level, the reason of the app, and what the browser host can do with it. The page also shows the versions, their sizes, their cores, and if they are signed.
- **Open:** runs the app in the web host (`index.html?app=…&core=…`) with the core from the registry. The app needs a core with the same major version and a minor version that is not lower (`SPEC.md` §10.5). A link above the app goes back to the list.

The risk levels and the descriptions come from the registry. `plinth registry build` writes them in the app list (`labels`, `docs/REGISTRY.md` §3) from the shared capability map (`crates/plinth-link/src/capabilities.rs`). The page does not have a copy of that table.

## 4. Capabilities in the browser

| Capability | In the browser |
|---|---|
| `store.kv` | Works. The data stays in the browser, for this site (`localStorage`). |
| `clipboard.write` | Works. |
| `clipboard.read` | Limited. The browser asks for permission. The first read can return nothing (the browser API is asynchronous; the host keeps the last value). |
| `net:<host>`, `net:*`, `net.local` | Limited. The browser sends the request only if the server allows requests from this page (CORS). |
| `hub.manage` | Not supported. Calls are refused (`denied(unsupported)`). Use the desktop Hub. |
| Other names | Not supported. Calls are refused. |

The page shows a warning on each app that needs a capability that the browser host does not support. The app opens, but these parts do not work.

## 5. The server

`plinth registry serve <folder> --web` serves:

| Path | What |
|---|---|
| `/` | A redirect to `/web/hub.html`. |
| `/web/…` | The web App Hub and the web host. These files are in the `plinth` binary (`crates/plinth-registry/src/web_files.rs`), so the server works from any folder. |
| Other paths | The files of the registry folder. |

- The server binds to `127.0.0.1` only. Another device can connect only through a tunnel.
- Each response has `Cache-Control: no-cache`, so the browser gets a rebuilt app or registry at once.
- `.wasm` files have the type `application/wasm`.
- The server refuses paths outside the folder (`..`, absolute paths, links to other folders).
- Without `--web`, the server serves only the registry folder (`docs/REGISTRY.md` §8).

## 6. Limits (draft 1)

- All apps use the same origin. Thus all apps share the browser storage of the site; the web host puts the app id before each key, but an app could read the keys of a different app. `docs/HUB.md` §7.6 asks for one origin for each app; that is future work.
- The page does not ask for consent. It shows the label, and the web host gives each declared capability that it supports.
- The page does not check the package digest or the signature. The desktop Hub does these checks (`docs/REGISTRY.md` §5).
- The registry has no `summary` or `description` from the manifest yet, so most apps show no summary.

## 7. Tests

- `cargo test -p plinth-registry --test web_hub`: the redirect, the types, `no-cache`, the embedded files, and the path checks.
- `node web/test/run-hub-logic.mjs`: the page logic (`web/hub-logic.js`) against a registry that `plinth registry serve --web` serves, and counter from the Open URL. `scripts/ci-local.sh` runs it.
- `node web/test/run-a11y.mjs` (by hand; it needs Edge and the axe-core CDN): the page in headless Edge, in light and dark mode and at phone width; Open runs counter; axe-core finds no serious problems. `--hub-only` runs only this part. `A11Y_SHOTS=<folder>` also saves screenshots.
