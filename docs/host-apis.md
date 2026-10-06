# Host APIs, capabilities, and the manifest

A Plinth app cannot reach the outside world by itself. Every effect
outside its own UI — storage, the clipboard, the network, even a modal
dialog — goes through a **host API module** (`plinth:time`,
`plinth:store`, `plinth:clipboard`, `plinth:dialog`, and, in progress,
`plinth:net`). Most of these modules are gated by a **capability**:
a named permission that the app must declare in `plinth.toml`, and that
the user grants before the app can use it.

## `plinth:time`

Clocks and timers. No capability is needed.

```ts
import { now, monotonicNow, setTimeout, setInterval, clearTimeout, clearInterval } from "plinth:time";
```

| Function | Notes |
|---|---|
| `now()` | Milliseconds since the Unix epoch, UTC. For display and logging only. |
| `monotonicNow()` | Monotonic milliseconds from an arbitrary origin; use it to measure elapsed time. Never goes backwards. |
| `setTimeout(callback, ms)` / `setInterval(callback, ms)` | Return a timer id. |
| `clearTimeout(id)` / `clearInterval(id)` | Canceling an unknown or already-fired timer is not an error. |

The desktop host polls timers every 15 ms and delivers one firing per
poll; it does not catch up missed ticks.

### Date and time (docs/GAPS.md gap #5)

```ts
import { timezoneOffset, dateParts, makeDate, formatDate, toISOString, parseDate } from "plinth:time";
```

| Function | Notes |
|---|---|
| `timezoneOffset(ms)` | The local time zone's offset from UTC, in minutes east of UTC, at the instant `ms`. From the OS (desktop) or `-Date.getTimezoneOffset()` (web). |
| `dateParts(ms, utc?)` | Breaks `ms` into `{ year, month, day, hour, minute, second, millisecond, weekday }`. `month` is 1-12; `weekday` is 0 (Sunday) to 6. Local time unless `utc` is true. |
| `makeDate(year, month, day, hour?, minute?, second?)` | The inverse of `dateParts`: local wall-clock fields to `ms`. `hour`/`minute`/`second` default to 0. |
| `formatDate(ms, pattern, utc?)` | A small pattern language: `YYYY MM M DD D HH H hh h mm ss SSS ddd dddd MMM MMMM A`; any other character passes through. English names only — locale-aware formatting is `plinth:locale` (SPEC.md §4.7), later. |
| `toISOString(ms)` | `ms` as a UTC ISO-8601 string, like JS's `Date.prototype.toISOString`. |
| `parseDate(text)` | Parses `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS[.sss]][Z\|±HH:MM]`. No `Z`/offset means local time. Returns `null` on no match. The same format `DatePicker` stores. |

The local offset is looked up at the naive ("as if UTC") instant for the
date being converted, so it can be off by the DST delta for a wall-clock
time that falls inside a transition — the same approximation most host
platforms make. The desktop runner's offset source is injectable for
tests (`Guest::set_fake_timezone_offset`); real-host tests only check
that it is a multiple of 15 minutes.

## `plinth:store`

A per-app key-value store. Needs the `store.kv` capability.

```ts
import { kv } from "plinth:store";

kv.set("note", "hello");
kv.get("note");     // "hello" | null
kv.remove("note");
kv.keys();           // string[], no particular order
kv.lastError();        // the reason the last call was denied, or null
```

On the desktop host, the store is a JSON file at
`%APPDATA%\plinthpps\<app id>\kv.json`.

## `plinth:clipboard`

The system clipboard. `writeText` needs `clipboard.write`; `readText`
needs `clipboard.read` — declare whichever ones you use.

```ts
import { writeText, readText, lastError } from "plinth:clipboard";

writeText("copied!");
readText();  // string | null (null if the clipboard holds something other than text)
```

## `plinth:dialog`

Host-owned modal dialogs. **No capability is needed** — a dialog is UI,
not data access. Each call returns at once; the user's answer arrives
later, through a `completion` event, and calls your `done` callback.

```ts
import { alert, confirm, prompt } from "plinth:dialog";

alert("Saved.");                                   // done is optional
confirm("Reset the count?", (ok) => { /* ok: boolean */ });
prompt("Your name?", (value) => { /* value: string | null */ });
```

The desktop host shows one request at a time as a modal overlay: `alert`
has an OK button; `confirm` has Cancel and OK; `prompt` adds a text
field. Escape cancels and Enter confirms. The web host uses the
browser's own `alert`/`confirm`/`prompt`.

## `plinth:net` — in progress

Planned `fetch`-like HTTP and WebSocket access, gated by
`net:<host-pattern>` capabilities (for example `net:api.example.com`, or
`net:*` for any host, which needs a stated reason). It is blocked on
`async`/`await` landing in the compiler (see [language.md](language.md)),
because a network call cannot finish synchronously. Do not depend on
`plinth:net` yet — it is not implemented.

## `plinth:hub` (privileged)

The Hub UI (`examples/hub`, `docs/HUB.md` §4.1) uses this module. It needs
the `hub.manage` capability (High risk). The host gives `hub.manage` only
to a package that a trusted Hub key signed (`PLINTH_HUB_TRUSTED_KEYS`); it
refuses every other package that declares it, before the package runs.

| Call | Core | What it does |
|---|---|---|
| `listApps()` | 1.7 | The library as JSON text (decode it with `JSON.parse<T>`): each app with its version, publisher, signer, source, groups, `blocked`, and its capability label (risk, description, reason, decision). `null` if denied. |
| `launch(id)` | 1.7 | The host opens the app in a new window, with the consent window first if necessary. |
| `setGrant(id, capability, allowed)`, `block(id)`, `unblock(id)` | 1.7 | Change a grant; block or unblock an app. |
| `listGroups()` | 1.8 | The groups as a JSON string array, or `null`. |
| `createGroup(name)`, `setGroup(id, group, member)`, `remove(id)` | 1.8 | Make a group; put an app in a group or take it out; remove an app from the library. |
| `search(query, done)` | 1.8 | Searches every configured source off the UI thread. `done` gets `{ hits, errors }` as JSON text, or `null` if denied. |
| `install(id, done)` | 1.8 | Installs the latest version from the first source that lists it. `done` gets `null` on success, or the error text. |
| `lastError()` | 1.7 | The reason the last synchronous call was denied, or `null`. |

`search` and `install` use the same request id and `completion` event as
`plinth:dialog`. The web host answers every `plinth:hub` call with
"unsupported" (the web Hub is phase H6).

## Uncaught errors

An exception that the app does not catch (SPEC.md §5.6) does not stop
the app. The guest sends the text, for example `Uncaught Error: no
network`, to the host through `error.report` (core 1.9; no capability).
The desktop runner logs it as an error and keeps it for tests
(`Guest::take_errors`). The web host calls `reportError`, which is
`console.error` by default. The app continues with the next event.

## Declaring a capability

Add a `[[capabilities]]` block to `plinth.toml` for each capability you
use, with a plain-English `rationale` — this is the text a consent
screen shows the user:

```toml
id        = "com.example.notes"
name      = "Notes"
version   = "0.1.0"
publisher = "example"

[[capabilities]]
name      = "store.kv"
rationale = "Save your notes on this device."

[[capabilities]]
name      = "clipboard.write"
rationale = "Copy a note to share it."
```

## Undeclared calls are compile errors

If your code calls a gated function without declaring its capability,
`plinth check` rejects it — the mistake never reaches a user:

```
app/main.tsx(8,16): error PL1007: this call needs the `store.kv` capability, which `plinth.toml` does not declare
  <Text>{kv.get("note") ?? "none"}</Text>
         ^^^^^^^^^^^^^^
  help: add `[[capabilities]]` with `name = "store.kv"` and a `rationale` to plinth.toml (SPEC.md §11)
```

## Denied calls never trap

Even with a declared capability, a host can still refuse a call at run
time (the user declined it, or the host does not support it). A denied
call never throws or traps your app — it fails quietly, in a fixed way:

| Module | On denial |
|---|---|
| `plinth:store` | `kv.get` returns `null`; `kv.set`/`kv.remove` do nothing; `kv.keys()` returns `[]`. |
| `plinth:clipboard` | `readText()` returns `null`; `writeText()` does nothing. |

Each denied module exposes `lastError()` (`kv.lastError()`,
`clipboard`'s module-level `lastError()`), which returns one of:

- `"denied:undeclared"` — the manifest does not declare the capability.
- `"denied:refused"` — the user declined it.
- `"denied:unsupported"` — this host does not implement the capability
  (for example, the web host's clipboard and store are currently
  stubs that always deny — see `web/README.md`).
- `null` — the last call to that module succeeded. The value clears on
  the next successful call, so check it right after a call whose result
  you need to trust.

Write your UI to treat a denied capability as a normal, visible state
(for example, "Clipboard access was not granted") rather than assuming
every call succeeds.

## `plinth validate`: the capability report

`plinth validate <file.plnt>` prints what a package declares and what
it can actually reach, computed directly from its Wasm imports — not
from trusting the manifest:

```sh
$ plinth validate dist/notes.plnt
capabilities:
  declared:
    store.kv - Save your notes on this device.
  reachable: store.kv
ok: dist/notes.plnt
```

- **declared** is read from `plinth.toml`/`manifest.toml`.
- **reachable** is computed by mapping each runtime function the app
  module imports to the capability it needs. This is the same check a
  hub or registry would run before listing the app (see
  [architecture.md](architecture.md) and `docs/HUB.md`): a package
  cannot hide a capability it can use, and a report of "declared but
  not reachable" flags a capability you asked for but never use.

A package with no capabilities (like the example in `plinth new`) prints
`declared: none` and `reachable: none`.
