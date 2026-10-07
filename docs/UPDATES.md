# Updates without a reinstall (design draft)

Status: draft from the owner's brainstorm of 2026-10-07. Nothing in this document is built yet, except the parts that "Today" names. Open questions are marked UP-Q*.

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

1. **Core patch versions.** A core fix that adds no function keeps its version today, so two different builds of `1.12` cannot be told apart. Cores need `MAJOR.MINOR.PATCH` and a digest; selection takes the highest patch.
2. **An author policy for cores** (in `plinth.toml`):
   - `core = "follow"`: take new patches and minors automatically (the current behavior).
   - `core = "patch"`: take only patches of the core the app was released with.
   - `core = "pinned"`: one core build, by digest.
3. **An update feed** for each distributable: a signed `feed.json` at a URL that the author chooses, with the app versions, the core each needs, and where to get packages and cores. The Hub's registry format is the start point (`docs/REGISTRY.md`).
4. **An updater in the host:** check the feed, download into the per-user data folder, verify, and use the newest set that the native host supports. The embedded `.plnt` and core stay as the fallback.
5. **A project key** that signs cores (HANDOFF open decisions: "the project Hub key").

## 5. The author's configuration (proposal)

```toml
[updates]
feed = "https://example.com/myapp/feed.json"
channel = "stable"     # or "beta"
check = "on-start"     # or an interval
apply = "next-start"   # "prompt" | "next-start" | "live"
core = "follow"        # "patch" | "pinned"
```

`apply = "live"` replaces a running app and keeps its state, with the same signal snapshot that hot reload uses (SPEC §13). It needs care when the shape of the state changes.

## 6. Safety

- **Signatures:** an app update must have the publisher key of the installed build; a core must have the project key. A digest check for every file.
- **No downgrade:** versions only go up (a feed cannot roll a user back to an old, unsafe version).
- **Rollback:** if a new set stops at start (a trap in `init`) two times, the host goes back to the previous set and reports it. The "This app stopped" path (2026-10-07) gives the signal.
- **Data:** the host stores the app version with the app's data; an `onUpgrade(fromVersion)` hook lets an app move its data. A rollback must not lose data that a newer version wrote (UP-Q5).
- **Capabilities:** an update that declares a new capability asks the user again, as the Hub does.

## 7. Platforms

- **Desktop:** first. A per-user install (as VS Code and Chrome do) means that no update needs administrator rights. A native update can also be a per-user self-update: download the new executable and swap it at the next start.
- **Web:** the same layers: `plinth.js` is the native layer, plus `core.wasm` and the `.plnt`, all cached by a service worker and keyed by digest.
- **iOS:** iOS does not permit JIT. **Decided (2026-10-07): iOS runs Wasm with the `wasmi` interpreter** (SPEC §9.2), so code that arrives after install runs the same way as embedded code. The App Store rules for downloaded, interpreted code are a separate question (UP-Q6); the owner wants the feature on every host anyway.
- **Android:** wasmtime permits JIT; the same updater as the desktop.

## 8. Order of work (proposal)

1. Core patch versions and the `core` policy (the other parts depend on them).
2. The feed format and its signature (reuse the registry).
3. The desktop updater for `plinth native`, with rollback.
4. The web service worker cache.
5. `apply = "live"`.
6. Mobile, with the `wasmi` runner (`plinth-runner-wasmi`).

## 9. Open questions

| # | Question |
|---|---|
| UP-Q1 | The default core policy: `follow` (users get core fixes with no action from the author) or `patch` (more control for the author)? Proposal: `follow`. |
| UP-Q2 | `apply = "live"` in the first version, or later? |
| UP-Q3 | Feeds only at the author's own URL, or also a Plinth-hosted registry service? |
| UP-Q4 | The project key that signs cores, and its rotation. |
| UP-Q5 | What a rollback does with data that the newer version wrote. |
| UP-Q6 | App Store policy for an app that updates its own `.plnt` (most defensible) and for the Hub on iOS (hard; an alternative marketplace in the EU). |
