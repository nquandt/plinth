# Plinth Hub — Specification

> Status: design (2026-10-06). Nothing in this document is built yet, except the parts that it marks as **exists**. `SPEC.md` is the main specification; this document expands `SPEC.md` §12 (App hub), §11 (security), and §18 (browse mode). It uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

---

## 1. Purpose

The **Plinth Hub** is the app that a non-technical person installs to get and use Plinth apps. It does four jobs:

1. **Find apps.** The user searches, browses, and opens apps from a registry or from a link.
2. **Keep apps.** The user adds apps to a library, puts them in groups, and gets updates.
3. **Run apps safely.** The Hub shows what each app can do (its capabilities) before it runs. The user approves, limits, or blocks it.
4. **Supply the runtime.** The Hub contains a Plinth host and installs the runtime cores (`SPEC.md` §10.5) that apps need.

The Hub is the same product on desktop (Windows, macOS, Linux), mobile (Android, iOS), and the web. An app that works in the Hub on one device works the same on every other device, because each device runs the same `.plnt` file on the same core version.

### 1.1 Why this is different from a web browser

A web page can try to do anything that the browser allows, and the user cannot see the list in advance. A Plinth app is different in a way that a machine can check:

- An app can reach the outside world **only** through the host APIs (`SPEC.md` §8.5).
- An app module can call a host API only through a runtime function that it **imports** (`SPEC.md` §10.4). The import list is in the file.
- Thus the Hub can read the app module and prove which capabilities the app **can** use, not only which capabilities the manifest **declares**. It shows both, and it refuses an app whose imports need a capability that the manifest does not declare.

The Hub turns this fact into a clear consent screen: "This app can save data on this device. It cannot use the network. It cannot read your files."

### 1.2 Goals

- A person with no technical knowledge can install the Hub, find an app, understand what it needs, and run it in less than one minute.
- One app file runs on every device. The developer builds it one time.
- The user is in control: every capability is visible, can be refused, and can be revoked. An app or a publisher can be blocked.
- Publishers are identified by keys. Published versions are signed and recorded in a public log, so a changed or hidden version is detectable.
- Apps are small (kilobytes) and start fast. The Hub caches them.
- The Hub works offline for the apps in the library.

### 1.3 Non-goals (for the first versions)

- Payments, subscriptions, or ads. (A later version can add them. The design must not prevent them.)
- Arbitrary native code in apps. Apps are Plinth TS only. Native code exists only in hosts.
- A replacement for the platform stores. The Hub can be distributed through them where their rules allow it (§11).

---

## 2. Ways to get and run an app

| Way | For whom | Runtime | Status |
|---|---|---|---|
| **Plinth Hub** (installed app) | everyone | installed one time with the Hub; cores installed on demand | design (this document) |
| **Browse mode** (open a link) | everyone with the Hub | the Hub's cores; the app is cached, not installed | design (`SPEC.md` §18) |
| **`plinth` CLI** (`plinth run app.plnt`) | developers | built into the CLI; `plinth core install` for more | **exists** |
| **`plinth-host`** runner | power users, scripts | built in | **exists** |
| **Native export** (`plinth native app.plnt -o App.exe`) | anyone without the Hub | the runtime is inside the executable | **exists** (current OS only) |
| **Web Hub** (a web site or a PWA) | everyone with a browser | the core file runs in the browser | early web host **exists** (`web/`) |

A native export is the answer when a person does not want the Hub: the developer (or the Hub, §9.4) makes one executable that contains a host, a core, and the app. A native export is specific to one platform. The `.plnt` stays universal.

---

## 3. Concepts

| Term | Meaning |
|---|---|
| **App** | A product with a stable **app id** (reverse domain, for example `com.example.notes`) that a publisher owns. |
| **Version** | One `.plnt` of an app (semver). It is immutable. Its identity is the SHA-256 of the package. |
| **Publisher** | A person or an organization that signs versions with a publisher key. |
| **Source** | A place where the Hub finds apps: an HTTPS endpoint that serves a signed index (§5). The default source is the Plinth registry. |
| **Library** | The apps that the user added. They stay on the device and work offline. |
| **Group** | A user-made collection in the library (for example "Work", "Family"). An app can be in more than one group. |
| **Cache** | Apps that the user opened in browse mode and did not add. The Hub can remove them (`SPEC.md` §18.2). |
| **Core** | A runtime build (`plinth-rt`), versioned `MAJOR.MINOR` (`SPEC.md` §10.5). |
| **Capability** | A permission that a host API needs (`SPEC.md` §11), for example `store.kv` or `net:api.example.com`. |
| **Grant** | The user's decision for one capability of one app: allowed always, allowed for this session, asked each time, or refused. |
| **Block** | The user's decision that an app, or every app of a publisher, must not run. |

---

## 4. Architecture

### 4.1 The Hub is a host plus a privileged Plinth app

The Hub has two layers:

1. **The Hub host** — the native part. It is the normal Plinth host for the platform (on desktop, `plinth-host-desktop` with gpui-ce), plus the Hub services: the library store, the package cache, the core store, the grants store, the source client, signature checks, and OS integration (shortcuts, URL scheme, file associations).
2. **The Hub UI** — a Plinth app written in Plinth TS (for example `dev.plinth.hub`). It uses the same controls as every other app, so it looks and behaves the same on every platform. It has one extra, privileged capability: `hub.manage`. The host gives `hub.manage` only to a Hub UI package that the Plinth project key signs.

`hub.manage` adds a host API module, `plinth:hub`, with calls such as `search(query)`, `appInfo(id)`, `install(id, version)`, `remove(id)`, `launch(id)`, `grants(id)`, `setGrant(id, capability, decision)`, `block(id | publisher)`, `groups()`, `addShortcut(id)`, `cores()`. Ordinary apps cannot import it: the import check of `SPEC.md` §10.4 rejects it unless the package has the Hub signature.

Reasons for this split:
- The Hub UI is written one time and runs on every host, like any Plinth app. It is also the largest test of the framework.
- The privileged part is small and native, so it is easy to audit.
- A developer can replace the Hub UI for a special device (a kiosk, a school device) without a change to the host.

### 4.2 Running apps

- **Desktop:** each app runs in its own window and its own guest instance. The Hub window is the launcher. The host can run more than one app at the same time (`SPEC.md` §18.4). Closing the Hub window does not close the apps.
- **Mobile:** each app runs full screen. The system back gesture goes to the app's navigation first, then to the Hub.
- **Web:** each app runs in its own browsing context (an iframe with a separate origin, or a separate tab), so its storage stays separate (§7.6).

Each app instance gets its own capability policy (from the grants) and its own data directory (by app id). Apps cannot see each other.

### 4.3 Local stores

All stores are per user and live in the Hub data directory (for example `%LOCALAPPDATA%\plinth\` on Windows):

| Store | Content |
|---|---|
| `packages/<sha256>.plnt` | Content-addressed packages (library and cache). |
| `cores/<major>.<minor>/core.wasm` | Installed cores. **Exists** (`plinth core`). |
| `library.json` | App ids, the pinned version or "latest", groups, order, the source of each app. |
| `grants.json` | Decisions for each (app id, capability), with the time and the version that the user saw. |
| `blocks.json` | Blocked app ids and publisher keys. |
| `apps/<app id>/` | App data (kv store **exists**; files later). |
| `sources.json` | The configured sources and their trusted keys. |
| `log/` | A local audit log: installs, updates, grant changes, denied calls. |

---

## 5. Discovery and the registry

### 5.1 Sources

> The exact format of a source is `docs/REGISTRY.md` (a NuGet-style service index at `<base>/plinth-registry.json`; static folders work).

A **source** is an HTTPS endpoint that serves:

- `index.json`: the apps that it lists, each with the app id, the publisher key, the versions (version, package digest, size, core version, UI API version, capabilities), the name, the description, the icon, screenshots, and categories. The source signs the index with its source key.
- Package files by digest: `/packages/<sha256>.plnt`.
- Core files by version: `/cores/<major>.<minor>/core.wasm`, signed by the Plinth project key.
- A search API (optional): `/search?q=…`.

The Hub has one default source, the **Plinth registry**. A user or an organization can add more sources (a company's internal apps, a community catalog). Each source has a name and a key. The Hub shows the source of each app.

Because packages and cores are content-addressed, a mirror or a CDN can serve them. The Hub checks every digest, so a mirror cannot change a file.

### 5.2 Search and browsing

- Search by name, description, category, and publisher.
- Each app page shows: the name, the publisher (with its verification level, §6.2), the capability label (§7.2), the size, the supported core version, screenshots (the registry makes them with `plinth-shoot` for each width class), the version history, the change log, and the reports state.
- Lists: new, popular, updated, categories, "works offline", "no network access".

### 5.3 Links

A Plinth app link opens the app in the Hub:

- `plinth://app/<app id>` (latest version from the user's sources),
- `plinth://app/<app id>@<version>`,
- `https://hub.plinth.dev/app/<app id>` (a web page that opens the Hub, or the web Hub when the Hub is not installed),
- a direct `.plnt` URL or file (the Hub shows that it does not come from a source).

A link that is not from a source gets a stronger warning on the consent screen (§7.3). The final choice of the app address is open question Q12 in `SPEC.md`.

---

## 6. Identity and authorship

### 6.1 Publisher keys and signatures

- A publisher creates a key pair with `plinth publisher init` (Ed25519). The private key stays with the publisher; a hardware key or an OS key store is supported later.
- `plinth publish` signs the package: `signature.json` in the `.plnt` holds the publisher key id and a signature over the digest of each other entry (`SPEC.md` §10.1).
- The registry co-signs each version that passes its checks (§8). The Hub shows "Signed by <publisher>, checked by <source>".
- The app id belongs to the first publisher key that publishes it in a source. A later version must have the same key, or a key that the old key has signed (key rotation).

### 6.2 Verification levels

| Level | How | Shown as |
|---|---|---|
| Unverified | a key only | "Unverified publisher" |
| Domain | the publisher puts its key id in `https://<domain>/.well-known/plinth-publisher` or a DNS TXT record, and the app id matches the domain | "Verified: example.com" |
| Organization | a manual check by the registry (later) | "Verified organization" |

An app id must match the verified domain (`com.example.*` needs `example.com`). This stops look-alike names for verified publishers.

### 6.3 Transparency log

Every version that a source publishes goes into an append-only, public log (a Merkle tree, as in Certificate Transparency or the Go checksum database). An entry holds the app id, the version, the package digest, and the publisher key. The Hub:

- checks that each version that it installs is in the log,
- remembers the log head that it saw, and refuses a log that is not an extension of it.

Thus a source cannot show one package to one user and a different package to another user without detection. A publisher can monitor the log for versions that it did not publish (a stolen key).

### 6.4 Revocation

- A publisher can revoke a key or a version.
- A source can withdraw a version (malware, legal reasons) with a reason.
- The Hub fetches the revocation list with the index and stops a revoked version. It shows the reason, keeps the app data, and offers an update or removal.

---

## 7. Capabilities and consent

### 7.1 What the Hub knows about an app

For each version, the Hub computes three lists:

1. **Declared:** the capabilities in the manifest, each with its rationale (`SPEC.md` §11).
2. **Reachable:** the capabilities that the app module's imports can use. The Hub maps each imported runtime function to the capability that it needs (for example `kv_get` → `store.kv`). The registry and the Hub compute this list from the file; nobody has to trust the publisher.
3. **Used at run time:** the capabilities that the app called, from the local audit log.

Rules:
- If an app can reach a capability that it does not declare, the source refuses to publish it and the Hub refuses to run it.
- If an app declares a capability that it cannot reach, the Hub does not ask for it and shows "declared but not used".

### 7.2 The capability label

Each app page and each consent screen has a short, fixed-format label, in plain words, with a risk level for each capability:

| Level | Examples | Default |
|---|---|---|
| **None** | UI only, `time` | allowed, no question |
| **Low** | `store.kv` (data on this device), `clipboard.write`, `notify` | allowed at install, shown in the label |
| **Medium** | `clipboard.read`, `net:<listed hosts>`, `fs.pick` (files that the user picks) | asked one time |
| **High** | `net:*` (any host), `fs.app-data` export, `camera`, `microphone`, `location` | asked each time, or for a session |

The label names each network host exactly ("This app connects to api.example.com"). The words and the levels come from one table in the host, so every app gets the same wording.

### 7.3 Consent flow

1. **Before the first run** the Hub shows the label: what the app can do, what it cannot do, the publisher, and the source. Low-risk capabilities are granted with the install. The user can refuse each one.
2. **At the first use** of a medium or high capability, the host pauses the call (an asynchronous host call, §12.1) and asks: "Allow <app> to read the clipboard? — Allow once / Always allow / Don't allow".
3. **On an update** that adds a capability, the Hub asks again before the new version runs. If the user refuses, the old version keeps running.
4. **In browse mode** (`SPEC.md` §18), grants last for the session by default (`SPEC.md` Q11).

A refused call returns `denied(refused)` and never traps (`SPEC.md` §8.5). Apps must handle it.

### 7.4 Control after install

- An app page in the library shows each grant, when the user gave it, and when the app last used it. The user can change or revoke each grant.
- The user can **block** an app (it does not start; its data stays), or block a **publisher** (all its apps and future apps).
- The user can **reset** an app (remove its data and grants) and **export** its data.
- A global switch can turn off a capability for all apps (for example "no apps may use the network").

### 7.5 Policies for managed devices

An administrator (a school, a company, a parent) can give the Hub a policy file: allowed sources, blocked capabilities, an allow-list of apps, and the update rule. The Hub shows that a policy is in effect.

### 7.6 Isolation

- Each app has its own guest instance, its own memory, and its own data directory.
- The web Hub runs each app in a separate origin, so `localStorage` and other storage stay separate.
- No capability lets one app read another app's data. A share capability (`plinth:share`, later) goes through a system picker that the user controls.

---

## 8. Publishing and review

`plinth publish` sends a signed package to a source. The source runs these checks before it lists the version:

1. **Structure:** the package rules of `SPEC.md` §10.1 (paths, sizes, zip safety).
2. **Module:** the app module checks of `SPEC.md` §10.4 (imports only from `plinth-rt`, no own memory or table, only passive data).
3. **Capabilities:** reachable ⊆ declared (§7.1); each declared capability has a rationale; network hosts are specific unless the publisher asks for `net:*` and gives a reason.
4. **Compatibility:** the core version and the UI API version exist.
5. **Screenshots:** `plinth-shoot` renders each screen at each width class. The screenshots go on the app page.
6. **Accessibility lint:** every interactive control has a label (`SPEC.md` §6.1); images have `alt` text.
7. **Size and start time:** the source measures them and shows them.
8. **Name checks:** the app id matches the publisher's verified domain, or it gets an "unverified" mark; names that are close to well-known apps need a manual check.

The checks are automatic. The registry can add a human review for apps with high-risk capabilities. Users can **report** an app; a report goes to the source.

The sandbox removes most of the risk of malware (an app cannot read files or run processes). The main remaining risks are deception (an app that asks for a password and sends it to a server it declares) and abuse of granted capabilities. The label, the network host list, and reports address them.

---

## 9. The library

### 9.1 Library and groups

- **Add to library** keeps the app on the device, available offline.
- **Groups**: the user creates groups, puts apps in them, sorts them, and pins favorites. A group can be shared as a list of app links (a "collection").
- **Recent**: the Hub lists apps that the user opened recently, including cached ones.

### 9.2 Updates

- The Hub checks the sources for new versions (on start, and daily).
- Default: an update without new capabilities installs automatically; an update with new capabilities waits for consent (§7.3).
- The user can pin a version, and roll back to an earlier version that is still in the cache.
- App data stays across updates. An app can migrate its own data on start.

### 9.3 Cores

- When an app needs a core that is not installed, the Hub downloads it from a source, checks the Plinth project signature, and installs it (`SPEC.md` §10.5). **The core store exists** (`plinth core install`).
- The Hub removes cores that no library app needs, after a time.
- The Hub host itself updates separately from the cores. A new host can run all cores of the major versions that it supports.

### 9.4 Export

From an app page, the user can make a **native export** for the current platform (the same as `plinth native`): one executable with the app, its core, and a minimal host. This lets a person give an app to someone who does not have the Hub. The export keeps the signature and shows the publisher on first start.

---

## 10. Operating system integration

| Feature | Windows | macOS | Linux | Android | iOS | Web |
|---|---|---|---|---|---|---|
| Desktop/home shortcut that starts one app | `.lnk` that runs `plinth-hub --run <app id>`, with the app icon | a small `.app` launcher bundle | a `.desktop` file | a pinned shortcut (`ShortcutManager`) | a Shortcuts action or a web clip (limited) | a PWA install of the app page |
| Start menu / launcher entry | yes | Launchpad (via the `.app`) | `.desktop` file | via shortcut | — | — |
| `plinth://` URL scheme | registry entry | `Info.plist` | `.desktop` `MimeType` | intent filter | URL type | registered protocol handler |
| Open `.plnt` files | file association | UTI | MIME type | intent filter | document type | file handler (PWA) |
| Notifications | via `notify` capability | same | same | same | same | Notifications API |

A shortcut starts the app directly, without the Hub window. The app still runs inside the Hub host, with its grants and its data.

---

## 11. Platforms

| Platform | Hub host | Notes |
|---|---|---|
| Windows, macOS, Linux | `plinth-host-desktop` on gpui-ce, wasmtime | The first target. Installers: MSI/MSIX, DMG (notarized), AppImage/Flatpak. |
| Web | `web/` host, the core in the browser (side-module loading) | The web Hub is a web site and a PWA. Apps run in separate origins. Early host **exists**. |
| Android | gpui-ce mobile fork (`SPEC.md` Q1) and wasmtime | Google Play policy on downloaded code must be checked: Wasm in a sandbox with no native code is the strongest case. Alternative stores and direct install are possible. |
| iOS | gpui-ce mobile fork, a Wasm interpreter or AOT (no JIT) | App Store rule 2.5.2 limits apps that download code that changes their features. This is the largest risk (`SPEC.md` Q3). Options: (1) the Hub on iOS runs only apps from the curated Plinth registry, presented as content; (2) alternative marketplaces where the law allows them; (3) per-app native iOS builds, which a developer makes on a Mac with Xcode from the `.plnt` (the startup project that Xcode needs is generated by `plinth native --target ios`). |

Rule for every platform: the Hub UI is the same Plinth app, and each app is the same `.plnt` file.

---

## 12. Changes to the runtime that the Hub needs

### 12.1 Asynchronous host calls

Consent at first use (§7.3) and the network API need host calls that wait. `SPEC.md` §8.5 already defines them: a call returns a request id, and a `completion` event carries the result. Plinth TS needs a way to wait: first callbacks (`net.fetch(url, (result) => …)`), then `async`/`await` (`SPEC.md` §4.5). This is a prerequisite for §7.3 step 2 and for `plinth:net`.

### 12.2 More than one app in one host

The desktop host runs one app today. The Hub host needs: several guest instances in one process, a window for each app, one shared engine, and per-app policies and data directories (`SPEC.md` §18.4).

### 12.3 The capability map

A table in `plinth-link` maps each runtime function to the capability that it needs. The Hub, the registry, and the compiler (`PL1007`) use the same table. **Exists:** `crates/plinth-link/src/capabilities.rs` (names, risk levels, descriptions); `split::reachable_capabilities` and `split::check_capabilities`; the desktop host refuses an app that can reach an undeclared capability; `plinth validate` prints the declared, reachable and unused capabilities.

### 12.4 Grants store and prompts

The host's `Policy` (**exists**, `plinth-runner-wasmtime`) gets its decisions from the grants store, and it can ask the Hub UI for a decision through an asynchronous call.

---

## 13. Threat model (summary)

| Threat | Defense |
|---|---|
| A malicious app reads private data | Sandbox; no API without a capability; capabilities that the file proves; consent; per-app isolation. |
| An app hides what it does | Reachable capabilities come from the imports, not from the manifest; network hosts are listed; the audit log shows denied and allowed calls. |
| A good app turns bad in an update | New capabilities need new consent; the transparency log; rollback; reports; revocation. |
| A stolen publisher key | Key rotation and revocation; the transparency log lets the publisher see versions that it did not publish. |
| A compromised source or mirror | Content addresses; publisher signatures; the transparency log; the Hub pins the log head. |
| A fake app that looks like a known one | Verified domains; app ids bound to domains; name checks; the publisher and the source are always shown. |
| A malicious core | Cores are signed by the Plinth project key; a core is Wasm too and runs in the same sandbox as the app. |
| Abuse of an allowed capability (for example spam notifications) | Per-capability rate limits in the host; one-tap revoke; reports. |

---

## 14. Developer experience

The developer path stays short:

```sh
npm create plinth@latest my-app      # or: plinth new my-app
cd my-app && npm run dev             # window with hot reload
plinth publisher init                # one time: make a publisher key
plinth publish                       # sign, check, upload to the registry
plinth native -o MyApp.exe           # optional: a standalone executable
```

No platform SDK is necessary for the Hub path. A developer needs Xcode on a Mac only to make a standalone iOS build (§11).

---

## 15. Phases

| Phase | Content | Depends on |
|---|---|---|
| **H0: local library** | Hub services in the desktop host: library, package cache, grants store, blocks; the consent screen (install-time label) in the desktop host; `plinth hub` CLI commands for testing. **Exists:** crate `plinth-hub` (`PLINTH_HUB_DIR`), `plinth hub add\|list\|run\|remove\|grants\|block\|unblock\|groups\|policy`, policies from grants, the consent window before the first run. Risk-level defaults (§7.2): `None`/`Low` capabilities are granted "by default" at install (`Hub::add_package`), recorded in `grants.json` with `by_default: true`; the consent window and `plinth hub grants <id>` show every declared capability with its risk level and whether the user or the default decided (`Hub::capability_report`); a Low one can still be refused with `plinth hub grants <id> refuse <capability>`. Re-consent on update (§7.3 step 3): each installed version's capabilities are kept in `library.json`; a new version's run asks consent only for the capabilities the previous version did not have (a grant does not depend on the version); if the user cancels, `Hub::runnable_version`/`package` picks the newest version whose capabilities are all decided (or the pinned one), so the previous version runs; `plinth hub update` prints the new capabilities. Global switches (§7.4): `plinth hub policy deny\|allow\|show <capability>`, a hub-wide deny list in `policy.json` that `Hub::policy_for` applies after the per-app grants. | — |
| **H1: signing** | `plinth publisher init`, signatures in `.plnt`, verification in the host, the capability map (§12.3), reachable-capability analysis. | H0 |
| **H2: sources** | The source format (§5.1), a static source that a developer can host on any web server, `plinth publish --to <dir>`, the transparency log. | H1 |
| **H3: Hub UI** | `plinth:hub` (privileged), the Hub UI as a Plinth app, more than one app per host, shortcuts and the URL scheme on Windows. | H0–H2, §12.2 |
| **H4: registry service** | The hosted registry with search, screenshots, reports, domain verification. | H2 |
| **H5: first-use prompts and network** | Asynchronous calls, `plinth:net`, runtime prompts. | §12.1 |
| **H6: web Hub** | The web Hub as a PWA, apps in separate origins. | H3, `SPEC.md` M5 |
| **H7: mobile Hubs** | Android, then iOS (with the store question resolved). | `SPEC.md` M6 |

`SPEC.md` M7 (Hub) covers H1–H4. `SPEC.md` M8 (browser and progressive loading) covers browse mode in the Hub.

---

## 16. Open questions

| # | Question | Notes |
|---|---|---|
| H-Q1 | Who runs the default registry, and under what rules? | A foundation-like model or a company. It affects trust and the review policy. |
| H-Q2 | The exact signing scheme | Ed25519 keys and a simple signature file, or Sigstore-style keyless signing with OIDC identities. |
| H-Q3 | The transparency log implementation | Run a separate log, or use an existing public log (for example Sigstore Rekor). |
| H-Q4 | How the Hub UI asks for consent while an app waits | A Hub sheet over the app window, or a system dialog. |
| H-Q5 | Payments | Out of scope now; keep a place in the app page and the manifest. |
| H-Q6 | App names and trademarks | A dispute process for names. |
| H-Q7 | iOS distribution | `SPEC.md` Q3. |
| H-Q8 | Shared app libraries (`hub:<publisher>/<lib>`, `SPEC.md` §12) | They are compiled into each app; they do not change the runtime model. |
| H-Q9 | Data sync between a user's devices | A later capability (`sync`), with end-to-end encryption. |
