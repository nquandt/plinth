# Guide: ship updates

This guide tells you how users get new versions of your app, and how you control the runtime core that your app runs on. It is for app authors. The design is in `docs/UPDATES.md`.

## 1. The two controls

| Control | In `plinth.toml` | What it decides |
|---|---|---|
| The core policy | `core = "follow" \| "patch" \| "pinned"` (a top-level field) | Which runtime core a host links your app into. |
| The update feed | the `[updates]` section | Where a `plinth native` executable looks for new versions of your app, and when it uses them. |

`core` is a top-level field. Do not put it in `[updates]`: `plinth build` then stops with "unknown field `core`".

```toml
id = "com.example.updates"
name = "Updates"
version = "0.1.0"
publisher = "example"
core = "follow"

[updates]
feed = "https://example.com/myapp/feed.json"
channel = "stable"
check = "on-start"
apply = "next-start"
```

This file is checked: `plinth build --sign` and `plinth feed add` accept it.

## 2. The core policy

Your app module names the lowest core that it needs (the manifest field `runtime`, for example `plinth-core/1.0`). The compiler finds this version from the core functions that your app uses. A host can have more than one core.

| `core` | The host links your app into | Use it when |
|---|---|---|
| `follow` (the default) | The newest core of the same major version, with a minor version that is not lower than the one your app needs. The highest patch of that minor. | Almost always. Core fixes reach your users with no action from you. |
| `patch` | Only a core with the same `MAJOR.MINOR` as the core that built your app (`core-built` in `manifest.toml`). The highest patch. | You want bug fixes, but no new minor version. |
| `pinned` | Only the exact core build that `plinth build` used, by its SHA-256 digest (the manifest field `core-digest`). | Your app depends on the exact behavior of one core build. |

> **NOTE:** a package built before `core-built` existed has no such field. For it, `patch` uses the lowest core that the app needs, which can be older than any core on the host. Build the app again to record `core-built`.

If the host has no matching core, the app does not start, and the message tells the user which core to install (`plinth core install <core.wasm>`).

An app that has no `core` field uses `follow`. The web host supports only `follow` today (`docs/UPDATES.md` §4 item 2).

## 3. The update feed

Self-update works today only for an executable that `plinth native` makes, on the desktop. `plinth run`, the web host and mobile do not use the feed. Apps in the Hub update from their registry instead (see [publishing.md](publishing.md)).

### 3.1 `[updates]`

| Key | Values | Default | Notes |
|---|---|---|---|
| `feed` | A URL | — (required) | The URL of `feed.json`. |
| `channel` | `"stable"`, `"beta"` | `"stable"` | The channel that the executable follows. |
| `check` | `"on-start"` | `"on-start"` | The host checks the feed in the background when the app starts. No other value exists yet. |
| `apply` | `"next-start"`, `"prompt"`, `"live"` | `"next-start"` | `next-start`: the new version runs from the next start. `prompt` is accepted, but the host does not show a prompt yet; it acts as `next-start`. `live` is not built: `plinth build` refuses it. |

Without `[updates]`, the app never checks for updates.

### 3.2 Publish a version

1. Make a publisher key, one time: `plinth publisher init --name "<your publisher>"`. The name must be equal to `publisher` in `plinth.toml`. See [publishing.md](publishing.md).
2. Build and sign the package:

   ```sh
   plinth build --sign
   ```

3. Add the package to the feed folder:

   ```sh
   plinth feed add dist/feed dist/<name>.plnt --channel stable
   ```

   The command writes `feed.json`, `feed.json.sig` (an Ed25519 signature over the exact bytes of `feed.json`) and a copy of the package, `<last part of the app id>-<version>.plnt`. Add `--with-core` to also copy the core and list it in the feed by its digest, for a host that does not have this core.

4. Upload all files of `dist/feed` to the folder of the `feed` URL. Keep them next to each other.
5. Make the executable one time: `plinth native dist/<name>.plnt -o <Name>.exe`. Give it to your users. Later versions arrive through the feed.

For the next version, change `version` in `plinth.toml`, and do steps 2 to 4 again. `plinth feed add` adds the new version and keeps all older entries. It never changes or removes an entry.

`plinth feed add` refuses:

- an unsigned package (run `plinth sign` first);
- a package that a different key signed, or whose `publisher` is not the name of your key;
- a package with a different app id from the feed.

### 3.3 What the host checks

The rules are fixed. You cannot turn them off.

- **Key continuity.** The feed and the package must have the publisher key of the package in the executable. A feed that a different key signed is refused, also if its signature is correct.
- **No downgrade.** The version must be newer than the embedded version and than the version already downloaded.
- **Digests.** The host checks the SHA-256 of each file against the signed feed. A core from the feed is accepted only by the digest that the feed lists (there is no project key for cores yet).
- **Rollback.** If a new version stops at start two times in a row, the host goes back to the embedded version and writes the reason to the log.

The host keeps downloaded versions in `%LOCALAPPDATA%\plinth\updates\<app id>` on Windows. No step needs administrator rights.

## 4. Things that do not exist yet

Plan your releases with these limits (`docs/UPDATES.md` §9.1, `docs/LIMITS.md`):

- A self-update that declares a new capability does not ask the user again. Do not add a capability in a self-update until this is built (`docs/PRODUCTION.md` S5).
- There is no `onUpgrade(fromVersion)` hook. Your app must read the data that an older version wrote. For `kv` data in JSON, handle a `null` result of `JSON.parse<T>` (see [storage.md](storage.md)).
- What a rollback does with the data that the newer version wrote is an open question (`docs/UPDATES.md` UP-Q5).
- There is no restart prompt (`apply = "prompt"` acts as `next-start`).
- The web host and mobile hosts have no updater.
