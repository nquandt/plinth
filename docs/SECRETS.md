# `plinth:secrets`: secure storage for credentials

> Status: 2026-10-27. Steps 1 to 5 of §7 are built (Windows and web). Step 6 (iOS) and checks on macOS and Linux are open. This document uses ASD-STE100 Simplified Technical English, as `SPEC.md` does. The first mention of the idea is in `docs/STORAGE.md` (concept 6).

## 1. The goal

An app that talks to a service needs to keep a token, an API key or a password. Today the only place is `plinth:store` `kv`, which is a plain JSON file. A secret must not be in a plain file. The host must keep it in the safest place that the platform offers, and it must not give it to any other app.

## 2. The API

```ts
import { set, get, remove, names, lastError } from "plinth:secrets";

await set("api-token", "abc123");     // Promise<void>
const t = await get("api-token");     // Promise<string | null>
await remove("api-token");            // Promise<void>
const all = await names();            // Promise<string[]>, sorted
```

- The capability is `secrets`. Its risk is **Low**: the store belongs to this app only, like `kv`, so the host grants it with no question. (The user can still see it in the Hub.)
- A name has 1 to 64 characters from `a-z A-Z 0-9 . _ -`. A value is a string of at most 16 KiB (UTF-8). An app has at most 64 secrets. A call that breaks a limit rejects with `Error("denied:limit")`; a bad name does so for `get` and `remove` too. Replacing a secret at the limit of 64 is allowed. The functions are in core 1.13 (not yet shipped).
- `get` resolves with `null` when the name does not exist. A missing name is not an error.
- A denied call rejects with `Error("denied:<reason>")`, as `plinth:notify` does. The reasons are `undeclared`, `refused`, `unsupported`, `limit` and `failed`. `failed` means the safe store itself failed (for example a locked keychain); its message never holds a value. A denied call never traps the app. `lastError()` gives the reason of the last denied call, or `null`.
- `unsupported` means this host has no safe store. The host must **not** fall back to a plain file. The app must then ask the user, or work without the secret.

## 3. The rules

1. **One app, one namespace.** The host builds the key from the app id and the instance (`docs/STORAGE.md`, "Instances": the instance is `default`) and the name. An app can never reach another app's secrets. The package digest does not matter: an update keeps the secrets. Another publisher with the same app id cannot exist (the registry enforces it).
2. **The host never logs a value.** Not in the log, not in the trace, not in an error message. Tests check this.
3. **Uninstall removes the secrets.** The host deletes all secrets of the app, with its data (STORAGE S13).
4. **No cross-device copy.** A secret stays on the device. Sync of app data (STORAGE S2) never carries secrets in 1.0. The user enters the secret again on a new device.
5. **The value is a string.** For binary data, the app encodes it (for example Base64).
6. **Secrets are not in the manifest.** `plinth.toml` has only `capabilities = ["secrets"]`.

## 4. Where each host keeps the secrets

| Host | Store | Notes |
|---|---|---|
| Windows | Credential Manager (`CredWrite`, `CredRead`) | Target name `plinth/<app id>/<instance>/<name>`. A blob holds at most 2560 bytes, so a value is cut into chunks of 2 KiB. |
| macOS and iOS | Keychain (generic password items) | Service `plinth.<app id>.<instance>`, account = name. On iOS the item class is `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`: it does not leave the device and it is not in the backup. |
| Linux | Secret Service (libsecret) | If no service runs, the call answers `unsupported`. |
| Web | WebCrypto key in IndexedDB | A browser has no keychain. The page makes one AES-GCM key per profile with `extractable: false`, and stores only the encrypted values. This protects against a disk copy of the profile, **not** against code that runs in the page. The apps run in sandboxed frames, so the page does the work for the frame and checks the capability again (like `notify`). |
| Tests | An in-memory backend (`SecretsBackend` test double) | |

The Windows backend writes the name of an item without uppercase letters (`Token` is the item `^token`), because the Credential Manager compares target names without case. It rejects a name that the runner would reject (for example one with `~`). It keeps a CRC-32 of each value, so that a crash in the middle of a replace gives an error and not a wrong value. It takes one lock for each call, because the Credential Manager loses writes that threads make at the same time. A secret that an earlier build wrote under a name with an uppercase letter is not found (core 1.13 is not shipped): the app sets it again. The header of `crates/plinth-host-desktop/src/secrets.rs` has the details.

The Rust crate `keyring` covers Windows, macOS, iOS and Linux with one API. Use it, behind the trait `SecretsBackend` in `plinth-runner-wasmtime`, so that the host state does not depend on it (PRODUCTION R5: the iOS host must build without `wasmtime`).

## 5. Wire format

Like `plinth:notify`, every call returns a request id at once, and the result arrives through a `completion` event (SPEC.md §8.4) as a 4-element list: `ok` (bool), `n` (s32), `text` (string), `error` (string, or null when `ok`).

| Call | `ok` | `n` | `text` |
|---|---|---|---|
| `set`, `remove` | true | 0 | `""` |
| `get` | true | 1 if found, 0 if not | the value, or `""` |
| `names` | true | the count | a JSON array of the names, sorted |
| any denied call | false | 0 | `""` (`error` = `"denied:<reason>"`) |

## 6. Capability and policy

- Name `secrets`, risk Low, description "keep passwords and tokens for this app".
- Functions: `secrets_set`, `secrets_get`, `secrets_remove`, `secrets_names` need `secrets`. `secrets_last_error` needs none (like `kv_last_error`).
- The Hub shows "keep passwords and tokens" in the app's permissions. The user can turn it off. When it is off, calls reject with `denied:refused`.

## 7. The build steps

Each step builds and tests alone (`HANDOFF.md` §8).

1. **Wire and runner.** WIT interface `secrets`, core functions, capability, policy, a `SecretsBackend` trait with an in-memory backend, on both engines. Test: a compiled test app on wasmtime and wasmi (like `tests/notify.rs`).
2. **Compiler.** `std/secrets.d.ts`, the module `plinth:secrets` (check, lower, codegen), PL1007 golden for an undeclared call.
3. **Desktop backend.** `keyring` in `plinth-host-desktop`; an example `examples/secrets-demo`.
4. **Web host.** WebCrypto + IndexedDB in `web/`, the frame bridge, a browser test (`web/test/run-secrets.mjs`).
5. **Docs.** `docs/host-apis.md`, `docs/LIMITS.md`, PRODUCTION (a new row), CHANGELOG.
6. **iOS.** The `keyring` Apple backend in `plinth-host-ios` (Mac work; `docs/MAC-HANDOFF.md` §5).

## 8. Tests

| Test | What it checks |
|---|---|
| Round trip | `set` then `get` returns the value; `names` lists it; `remove` then `get` returns `null`. |
| Isolation | Two apps with the same secret name do not see each other's value. |
| Limits | A bad name, a value over 16 KiB, and the 65th secret reject with `denied:limit`. |
| Undeclared | An app without `secrets` fails to compile (PL1007). A host that denies the capability rejects with `denied:refused`. |
| No plain file | After `set`, the app data folder has no file that contains the value. |
| No log | The value is not in any log line. |
| Both engines | The same results on wasmtime and wasmi. |
| Web | The value is not in IndexedDB as plain text; a second app does not read it; a reload keeps it. |
| Hostile input (runner) | `crates/plinth-runner-wasmtime/tests/secrets_hostile.rs`. Random and hostile names and values (NUL, `/`, `..`, `~`, `__index__`, many scripts, 64 and 65 characters, 16 KiB and 16 KiB + 1, 64 and 65 secrets) are compared with a model. No panic. The limits hold exactly. Many apps with the same names stay apart. `names` is always sorted and equal to the items. A backend that fails puts no value and no backend text in a completion, and a failed call does not change the store. A denied call never reaches the backend. Fixed seed; `PLINTH_FUZZ_ITERS` (default 3000) and `PLINTH_FUZZ_SEED` set a long run. |
| Hostile input (compiled app) | `crates/plinth-compiler/tests/secrets.rs`, `compiled_hostile_names_and_values_never_trap`: bad names reject with `denied:limit`; NUL, an emoji and a lone surrogate in a value do not trap; the edges of 16 KiB. Both engines. |
| Windows backend, with a store double | `crates/plinth-host-desktop/src/secrets_double_tests.rs`. The backend runs on an in-memory store that can fail after N writes, ignores case and refuses an item over 2560 bytes. Values of 0 to 16 KiB, at every chunk edge (2047, 2048, 2049, 4096, ...), with 2-, 3- and 4-byte characters over the edge. A shorter value leaves no old chunk. A crash at every point of a replace, a new secret and a remove never gives a wrong value and never leaves an item that the index does not list. A name with `~`, `^` or `/` cannot reach the chunk of another secret. `Token` and `token` are two secrets. A random run against a model (`PLINTH_FUZZ_ITERS`, default 400). |
| Windows backend, real keychain | `crates/plinth-host-desktop/src/secrets.rs` tests, with unique test app ids and cleanup: round trip, isolation, 16 KiB, `Token` and `token`, values at the chunk edges, threads at the same time. They skip when the host has no keychain. |
| Drift of the host traits | `crates/plinth-runner-wasmtime/tests/host_traits_drift.rs` compares `wit/plinth/app.wit` with `src/host_traits.rs` in the default build (PRODUCTION R5). |

Not tested: a crash of the process between two real keychain calls (the store double simulates it); two Plinth processes that write the same app at the same time (the lock is per process); macOS, Linux and iOS backends.
