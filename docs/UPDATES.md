# Updates without a reinstall (design draft)

Status: draft from the owner's brainstorm of 2026-10-07. §9 step 1 (core patch versions and the author's `core` policy, SPEC.md §10.5) and steps 2-3 (the signed feed format and the desktop updater, with rollback) are done, 2026-10-07. Re-consent for new capabilities and the desktop restart prompt (`apply = "prompt"`) are done, 2026-10-07 (§6, §9.1). The rest of this document is still a design draft. Open questions are marked UP-Q*.

## 1. Goal

A Plinth distributable updates itself without a reinstall: a native export (`plinth native`, an `.exe` today, mobile builds later), a cached web app, and the Hub. The user installs the native part one time. After that, a new app version or a new core arrives as data. A new native build is necessary **only** when the native interop changes (a new host interface). On the desktop, no update after the first install needs administrator rights.

App authors decide when their app takes a new core. Plinth maintainers ship cores. The mechanism is built into the host and configured by the author; an author writes no update code.

## 2. The three layers

| Layer | What | Shipped by | Its dependency, as declared today |
|---|---|---|---|
| Native host | The executable: renderer, host APIs (the WIT interfaces), the Wasm engine | Plinth maintainers; for a `plinth native` build, the app author | — |
| Core | `core.wasm`, one build of `plinth-rt`, the same file on every platform | Plinth maintainers | A host checks the imports of a core and uses only a core whose interfaces it supports (SPEC §10.5) |
| App | `.plnt`, app code only | The app author | The lowest core version that has every function the app imports (`rt_abi::ADDED_IN`) |

So the update cases are:

1. **App only:** the new app needs no newer core than the installed one.
2. **Core only:** the new core needs no host interface that the native host lacks (a bug fix, or new functions that use existing interfaces).
3. **Core and app:** the app needs a newer core, and that core needs no new interface.
4. **Native:** a core needs a new host interface. Only this case needs a new native build (on mobile: a store update).

## 3. Today

- Cores are versioned `MAJOR.MINOR`. Inside a major version a core only adds functions; an app declares its lowest core (SPEC §10.5).
- A host keeps several cores and picks, for each app, the highest minor version of the app's major version.
- A host checks a core's imports before it uses the core.
- The Hub (`docs/HUB.md`) installs and updates apps from registries, with digest and Ed25519 checks, re-consent for new capabilities, and pins.
- A `plinth native` executable holds one `.plnt` and runs it on the cores built into the host. It does not update.

## 4. What is missing

1. **Core patch versions. Done (2026-10-07).** Cores carry `MAJOR.MINOR.PATCH` and a SHA-256 digest (`plinth-link::cores`); selection takes the highest minor, then the highest patch. `plinth core list` shows version and digest.
2. **An author policy for cores. Done (2026-10-07)**, in `plinth.toml` and carried into the manifest field `core` (SPEC.md §10.5):
   - `core = "follow"`: take new patches and minors automatically (the current behavior, and the default).
   - `core = "patch"`: take only patches of the core the app was released with.
   - `core = "pinned"`: one core build, by digest (recorded in the manifest at build time).

   The desktop host honors the policy when it links a package. Still open: the web host (today it always follows, SPEC §10.3) and the Hub's update check against a pinned app (it should refuse an update that would change the pinned core, `UP-Q1` follow-up).
3. **An update feed. Done (2026-10-07).** A signed `feed.json` + `feed.json.sig` at a URL the author chooses (`plinth-package::feed`, §7 below). `plinth feed add` writes it from a built, signed package.
4. **An updater in the host. Done (2026-10-07)**, for the desktop (`plinth-host-desktop::updater`): checks the feed in the background on start, downloads and verifies a newer version into the per-user updates folder, and runs the newest verified set from the next start on. The embedded `.plnt` stays the fallback. Still open: the web service worker cache (§9 step 4) and mobile (§9 step 6); a core fetched through the feed is accepted only by the digest the signed feed lists, because the project core key (item 5 below) does not exist yet (UP-Q4).
5. **A project key** that signs cores (HANDOFF open decisions: "the project Hub key"). Still open.

## 5. The author's configuration (built, 2026-10-07)

```toml
core = "follow"        # "patch" | "pinned"; a top-level field, not in [updates]

[updates]
feed = "https://example.com/myapp/feed.json"
channel = "stable"     # or "beta"
check = "on-start"     # the only policy built; an interval is future work
apply = "next-start"   # "prompt" | "next-start" | "live"
```

`ProjectConfig` (`plinth.toml`) carries this into the manifest's `updates` field unchanged (`plinth_package::UpdatesConfig`). An old `plinth.toml`, or a manifest built before this field existed, has no `updates`: the app never checks for updates. `apply = "live"` is rejected at `plinth build` time (not implemented, UP-Q2).

`apply = "prompt"` (built, 2026-10-07, desktop host): when the feed check of this start stores a newer version, the host shows a small banner at the top of the app window: "An update is ready. Restart now?", with "Restart" and "Later". The host owns this banner, as it owns the error banner (`PlinthRoot::show_update_prompt`). The banner is not modal: the app stays usable. "Restart" closes the app and starts it again in the same process, on the new version; if the new version adds a capability, the consent window comes first (§6). "Later" removes the banner; the new version then runs at the next start, as with `next-start`. The host does not check the feed again after a restart in the same process.

`apply = "live"` replaces a running app and keeps its state, with the same signal snapshot that hot reload uses (SPEC §13). It needs care when the shape of the state changes.

## 6. Safety (built, 2026-10-07, in `plinth-host-desktop::updater`)

- **Signatures:** an app update must have the publisher key of the *embedded* package (key continuity) -- a feed validly signed by a different key is refused outright, the same as a corrupt feed, not treated as "unsigned". A core must have the project key; until that key exists (item 5, UP-Q4), a core is accepted only when its digest is the one the already-verified, signed feed lists.
- **No downgrade:** versions only go up (`plinth_package::is_newer`); a feed cannot roll a user back to an old, unsafe version. The comparison is against the newest version already downloaded, or the embedded version if nothing has been downloaded yet.
- **Rollback:** if a new set stops at start (a trap in `init`, or the load itself failing) two times in a row, the host goes back to the embedded set and reports it (the log, and `updater::rollback_report` for the error banner path). One failure does not roll back; a success in between resets the count. The "This app stopped" path (2026-10-07) gives the per-start signal (`open_app_reporting_init`/`run_reporting_init`).
- **Data:** the host stores the app version with the app's data; an `onUpgrade(fromVersion)` hook lets an app move its data. A rollback must not lose data that a newer version wrote (UP-Q5). **Still open:** this hook does not exist yet.
- **Capabilities (built, 2026-10-07):** an update that declares a new capability asks the user again, with the Hub's rule (`docs/HUB.md` §7.3 step 3; the shared functions are `plinth_hub::added_capabilities`, `undecided` and `is_granted`). The updater stores a new version as `pending.plnt`. At the next start, `updater::prepare_start` compares its capabilities with the capabilities of the embedded package (the user accepted them at install) and with the decisions that the user made for earlier updates (`state.json`, `grants`):
  - No added Medium or High capability without a decision: the new version runs with no question. A Low or None capability is granted by default, as in the Hub. A removed capability needs no consent.
  - Else the host shows the consent window (`consent::show`) for the new capabilities only, each with its risk and the app's reason. **Continue** records each decision and runs the new version; a capability set to "Don't allow" is refused in the policy, and the app gets `denied` for it (`updater::refused_for_run`). **Cancel** keeps the previous version (the last accepted download, else the embedded package).
  - **After Cancel, the host does not ask again for the same version** (`declined-version` in `state.json`), and the feed check does not download it again. A newer version replaces it, and the host asks again. We chose this so that a user who said no is not asked at each start.
  - A decision does not depend on the version: a later version with the same capability runs with no question.

## 7. The feed format (built, 2026-10-07)

`plinth-package::feed` (`crates/plinth-package/src/feed.rs`). Two files, next to each other at the author's chosen URL:

- `feed.json` -- the data.
- `feed.json.sig` -- a **detached** Ed25519 signature over the exact bytes of `feed.json`.

**Why detached, not an envelope.** `signature.json` (`docs/HUB.md` §6.1) signs a *statement* (a digest of each package entry) that is a different value from the stored file, so it needs canonical JSON to make signing and verifying agree on the exact bytes. A feed has no such problem: the file an author writes to disk is the exact message to sign, so Ed25519 signs it directly, with no canonicalization step to get wrong on either side. This is the simplest option that is still secure, not a shortcut: signing a raw byte string is the ordinary use of the primitive. The downside -- two files instead of one -- is minor net of what it removes.

`feed.json`:

```json
{
  "schema": "plinth.feed/1",
  "id": "com.example.notes",
  "channels": {
    "stable": [
      {
        "version": "1.2.0",
        "package": "notes-1.2.0.plnt",
        "sha256": "9f2c...",
        "core": "1.2",
        "core-source": { "url": "core-1.3.wasm", "version": "1.3", "sha256": "ab12..." }
      }
    ]
  }
}
```

- `id` must equal the running app's manifest id; a host refuses a feed for a different app (a mismatched URL, or a stale copy).
- `channels` maps a channel name to its versions, newest first.
- `package` and `core-source.url` are relative to the feed's own URL, or absolute -- the same resolution rule as the registry (`docs/REGISTRY.md` §2).
- `core` is the app's lowest required core version (SPEC.md §10.5), the same value `plinth registry build` writes to an app document.
- `core-source` is present only when the author ran `plinth feed add --with-core`; a host that already has a compatible core ignores it.

`feed.json.sig`:

```json
{
  "schema": "plinth.feed/1",
  "publisher": "Acme",
  "key": "ed25519:...",
  "signature": "base64 over the raw feed.json bytes"
}
```

`plinth feed add <feed-dir> <app.plnt> [--channel <name>] [--with-core]` (`crates/plinth-cli/src/feed_cmd.rs`) builds or updates `feed.json`/`feed.json.sig` from a signed package, with the loaded publisher identity (`plinth publisher init`). It refuses an unsigned package, or one signed by a different identity than the one loaded, and refuses to add a version to a feed that is for a different app id. Running it again for a newer version keeps every earlier entry; it never edits or removes one.

## 8. Platforms

- **Desktop:** first. A per-user install (as VS Code and Chrome do) means that no update needs administrator rights. A native update can also be a per-user self-update: download the new executable and swap it at the next start.
- **Web:** the same layers: `plinth.js` is the native layer, plus `core.wasm` and the `.plnt`, all cached by a service worker and keyed by digest.
- **iOS:** iOS does not permit JIT. **Decided (2026-10-07): iOS runs Wasm with the `wasmi` interpreter** (SPEC §9.2), so code that arrives after install runs the same way as embedded code. The App Store rules for downloaded, interpreted code are a separate question (UP-Q6); the owner wants the feature on every host anyway.
- **Android:** wasmtime permits JIT; the same updater as the desktop.

## 9. Order of work (proposal)

1. Core patch versions and the `core` policy (the other parts depend on them). **Done (2026-10-07).**
2. The feed format and its signature (reuse the registry). **Done (2026-10-07).**
3. The desktop updater for `plinth native`, with rollback. **Done (2026-10-07).**
4. The web service worker cache.
5. `apply = "live"`.
6. Mobile, with the `wasmi` runner (`plinth-runner-wasmi`).

### 9.1 What is open, after steps 2-3

- **`plinth run`** (running a `.plnt` directly, not a single-file export) does not check for updates; only a `plinth native` executable's embedded-payload path (`plinth_host_desktop::run_payload`, which both `plinth` and `plinth-host` call) does. This matches `docs/UPDATES.md` §1: the update mechanism is for a distributable the user already installed, not for a package a developer points the CLI at directly.
- **`apply = "prompt"`: done (2026-10-07)** on the desktop host (§5). Tests: `updater::prompt_for` (unit), and the banner in `plinth-shoot` `tests/update_prompt.rs` (headless). A click on "Restart" in a live window is not tested automatically.
- **Capability re-consent on update: done (2026-10-07)** (§6 "Capabilities"). Tests: `updater::update_consent` and `refused_capabilities` (unit), and the state steps (ask, Cancel, no second question, a newer version, Continue, "Don't allow") in the `updater` tests. The consent window is the Hub's window, which is not tested in a live window.
- **`plinth-host`** (the stub that `plinth native` uses when it is next to the CLI) now runs the updater too (`plinth_host_desktop::run_payload`). Before, only the `plinth` executable did.
- **`onUpgrade(fromVersion)`** (§6 "Data", UP-Q5) does not exist.
- **The project core key** (item 5 of §4, UP-Q4) does not exist; a core from the feed is accepted only by the digest the signed feed lists.
- **The web host and mobile** (§9 steps 4 and 6) are untouched.

## 10. Open questions

| # | Question |
|---|---|
| UP-Q1 | **Still open in part.** The default core policy is implemented as `follow` (SPEC.md §10.5, 2026-10-07): users get core fixes with no action from the author. Open: whether `patch` should be the default instead, for more author control, once real-world fixes show how often `follow` matters. |
| UP-Q2 | **Decided (owner, 2026-10-07): after 1.0.** `next-start` and `prompt` come first. |
| UP-Q3 | **Decided (owner, 2026-10-07):** feeds at the author's own URL and in the registries that the user adds. No Plinth-run service. |
| UP-Q4 | **Decided (owner, 2026-10-07):** an offline root key that the owner holds signs a release key for CI. The root key rotates or revokes the release key. |
| UP-Q5 | What a rollback does with data that the newer version wrote. |
| UP-Q6 | App Store policy for an app that updates its own `.plnt` (most defensible) and for the Hub on iOS (hard; an alternative marketplace in the EU). |
