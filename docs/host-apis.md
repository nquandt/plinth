# Host APIs, capabilities, and the manifest

A Plinth app cannot reach the outside world by itself. Every effect
outside its own UI — storage, the clipboard, the network, even a modal
dialog — goes through a **host API module** (`plinth:time`,
`plinth:store`, `plinth:clipboard`, `plinth:dialog`, `plinth:net`,
`plinth:files` and `plinth:notify`). Most of these modules are gated by a **capability**:
a named permission that the app must declare in `plinth.toml`, and that
the user grants before the app can use it.

## `plinth:time`

Clocks and timers. No capability is needed.

```ts
import { now, monotonicNow, setTimeout, setInterval, clearTimeout, clearInterval, onFrame, cancelFrame } from "plinth:time";
```

| Function | Notes |
|---|---|
| `now()` | Milliseconds since the Unix epoch, UTC. For display and logging only. |
| `monotonicNow()` | Monotonic milliseconds from an arbitrary origin; use it to measure elapsed time. Never goes backwards. |
| `setTimeout(callback, ms)` / `setInterval(callback, ms)` | Return a timer id. |
| `clearTimeout(id)` / `clearInterval(id)` | Canceling an unknown or already-fired timer is not an error. |
| `onFrame(callback)` | Core 1.12. Calls `callback(dt)` before each frame that the host draws, with `dt`, the milliseconds since the previous frame (0 for the first). Returns a timer id. Use it for motion and games: move by `speed * dt`. |
| `cancelFrame(id)` | Stops a frame timer. |

The desktop host wakes at the next timer deadline (and at least every
15 ms, for net results) and fires every due timer. A late repeating timer
does not catch up missed ticks: after a stall it fires one time and then
keeps its period. Frame timers do not use this loop: the desktop host fires them in the gpui frame (it asks for the next
frame while a frame timer runs), and the web host uses
`requestAnimationFrame`. Both hosts send no frames while the window is
hidden or minimized (the browser stops animation frames in a hidden tab),
so `dt` can be large after the window comes back: limit it (Pong takes at
most 50 ms in one step).

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
`%APPDATA%\plinth\apps\<app id>\kv.json`.

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

// Without `done`, each call returns a Promise (core 1.10, language.md
// "Async functions and `await`"):
const ok = await confirm("Reset the count?");      // boolean
const name = await prompt("Your name?");           // string | null
await alert("Saved.");
```

The desktop host shows one request at a time as a modal overlay: `alert`
has an OK button; `confirm` has Cancel and OK; `prompt` adds a text
field. Escape cancels and Enter confirms. The web host uses the
browser's own `alert`/`confirm`/`prompt`.

## `plinth:net`

HTTP `fetch` (SPEC.md §4.7, core 1.5), gated by `net:<host>` capabilities
(for example `net:api.example.com`, or `net:*` for any host, which needs
a stated reason; a private or loopback address also needs `net.local`).
`fetch(url, options, done)` calls `done` with `{ ok, status, text, error }`.
Without `done`, `fetch(url, options)` returns a `Promise<Response>` for
`await` (core 1.10). A denied or failed request never throws and never
rejects: `ok` is `false` and `error` gives the reason. WebSocket is not
implemented.

## `plinth:files`

Text files in the app's **private space** (core 1.11, `docs/STORAGE.md`
§2 item 3 and §3). Needs the `files.private` capability. Its risk is Low
("save files on this device"): the Hub grants it with no question, as it
does `store.kv`. Only this app can read its private space.

```ts
import { read, write, list, stat, remove } from "plinth:files";

await write("notes/today.md", "# Today");   // makes the folder "notes"
const text = await read("notes/today.md");  // string
const entries = await list("notes");        // FileEntry[]: { name, kind, size }
const entry = await stat("notes/today.md"); // FileEntry | null
await remove("notes");                      // a file, or a folder and its contents
```

| Call | Result | Notes |
|---|---|---|
| `read(path)` | `Promise<string>` | The whole file. |
| `write(path, text)` | `Promise<void>` | Replaces the file. Makes the parent folders. |
| `list(dir)` | `Promise<FileEntry[]>` | Sorted by name. `""` is the root. `kind` is `"file"` or `"dir"`; `size` is the size of a file in UTF-8 bytes (`0` for a folder). |
| `stat(path)` | `Promise<FileEntry \| null>` | `null` if nothing is at `path`. |
| `remove(path)` | `Promise<void>` | A path that does not exist is not an error. |

Each call also has a callback form: the last argument is `done(error,
value)` (`done(error)` for `write` and `remove`); `error` is `null` on
success.

**Errors.** A failed call rejects the promise with an `Error`. Its
`message` is one of: `"denied:undeclared"`, `"denied:refused"`,
`"denied:unsupported"`, `"invalid-path: <rule>"`, `"not-found"`,
`"not-a-file"`, `"not-a-directory"`, `"not-text"` (the file is not UTF-8),
`"too-large"` (more than 8 MiB), `"quota"` (the space is full) or `"io:
<detail>"`. A failed call never traps the app; an uncaught rejection is
reported like any other (see "Uncaught errors").

**Folders.** A folder exists only while it holds a file: `write` makes the
parent folders, and when `remove` takes the last file out of a folder, the
empty folders go too. There is no `mkdir`. (The web host has no real
folders, so this keeps both hosts the same.)

**Path rules** (`docs/STORAGE.md` §3 rule 1). The host checks each path
itself; it does not trust a check in the app. A path is relative, with `/`
between segments, for example `notes/2026/today.md`. The host refuses:

- an empty path (except `""` for `list`), a path that starts with `/`,
  and a backslash anywhere;
- an empty segment (`a//b`, `a/`), and the segments `.` and `..`;
- control characters (U+0000–U+001F, U+007F–U+009F), NUL included;
- the characters `: * ? " < > |` (so no drive letters such as `C:` and no
  Windows streams such as `file:stream`);
- a segment that ends with a dot or a space;
- the Windows device names `CON`, `PRN`, `AUX`, `NUL`, `COM0`–`COM9`,
  `LPT0`–`LPT9` (also with an extension, for example `nul.txt`);
- a path longer than 1024 UTF-8 bytes, a segment longer than 255 bytes,
  or more than 32 segments.

The rules are the same on every host, also where a file system would
accept a name, so that a space can move between hosts. The host does not
decode a path: `%2e%2e` is a plain name. The host does not normalize
Unicode (there is no normalization crate in the workspace, and the core
must stay small): the refused characters are all ASCII, and NFC cannot make
or remove an ASCII character, so normalization cannot open a way out of the
space. Two spellings of one name (NFC and NFD) can be two files on one host
and one file on another (macOS); write names in NFC. File names are
case-sensitive on the web and not on Windows and macOS.

**Where the files are.**

- Desktop: `<data dir>/spaces/private/<owner>/<app id>/` (the data dir is
  `%APPDATA%\plinth` on Windows). `<owner>` is `key-<hash of the publisher
  key>` for a signed package, so a package with the same id from another
  publisher gets another folder (`docs/STORAGE.md` §3 rule 2), or
  `pkg-<package digest>` for an unsigned package (a new build is a new
  space), or `dev` for `plinth dev`. The app id is in lower case. The calls
  of one app run on one worker thread, in the order the app made them,
  never on the UI thread.
- Web: in a sandboxed app frame (the web Hub, a web export), the page keeps
  the files in its IndexedDB (database `plinth-files`), one namespace for
  each app id; the frame asks for each call through the `files-*` bridge
  messages (`web/README.md`). On the stand-alone page and in Node tests:
  in memory.

**Limits.** 8 MiB for one file; 50 MiB for the private space of one app
(the sum of the file sizes).

## `plinth:notify`

System notifications (core 1.12, `docs/PRODUCTION.md` H3). Needs the
`notify` capability. Its risk is Medium ("show notifications"): the Hub
asks the user one time. The host shows the notification in the system
notification area, outside the app window.

```ts
import { show, cancel, lastError } from "plinth:notify";

const result = await show({ title: "Reminder", body: "Your 5 seconds are up.", tag: "reminder" });
// result is "shown", or "unsupported" when this host has no notification area.
cancel("reminder");              // removes the notification with this tag
```

- `show({ title, body?, tag? })` returns a `Promise<NotifyResult>`
  (`"shown" | "unsupported"`). Write the options as an object literal. A
  new notification with the same `tag` replaces the old one.
- `cancel(tag)` removes the notification with `tag`. A tag that the host
  does not show is not an error.
- A denied `show` rejects with `Error("denied:<reason>")`. A denied
  `cancel` does nothing; `lastError()` then gives the reason.
- "unsupported" is not an error: show the message in the app window
  instead. `examples/notify-demo` does this.

**Where the host shows them.**

- Windows 10 and later: a toast in the notification center (WinRT
  `ToastNotificationManager`, `plinth-host-desktop/src/notify.rs`). The
  host registers an AppUserModelID for each app under
  `HKCU\Software\Classes\AppUserModelId\Plinth.<app id>`, with the app
  id as the display name. The tag goes to Windows with at most 64
  characters. If Windows refuses the toast, `show` answers "unsupported"
  (the log has a warning).
- Linux and macOS: not yet. `show` answers "unsupported".
- Web: the browser's `Notification` API (`browserNotify` in
  `web/plinth-web.js`). An app runs in a sandboxed frame with an opaque
  origin, which cannot use the API, so the frame sends the call to the page
  (`notify-show`, `notify-cancel`, `web/frame-host.js`). The page checks
  the capability again and shows the notification with the app's title,
  body and tag (the browser replaces an earlier one with the same tag);
  `cancel(tag)` closes this app's open notification with that tag. A
  stand-alone page (`web/index.html`) uses the API in the page itself.
  The permission:
  - "granted": `show` answers "shown".
  - "denied" (the user or the browser blocked notifications for the page):
    `show` rejects with `Error("denied:refused")`.
  - "default": the page asks (`Notification.requestPermission()`). If the
    browser does not ask, or the user closes the question, `show` answers
    "unsupported" (the console has a warning).
  - A user gesture: a click in the app frame also activates the page
    (Chromium's user activation goes to the parent frames; `run-a11y.mjs`
    checks it in Edge). But the activation ends after a few seconds, and
    `show` often comes later (`notify-demo` waits 5 s). Chromium asks
    without a gesture; Firefox and Safari ask only after a gesture, so
    there the first `show` without one answers "unsupported". Headless
    Edge does not ask: it answers "denied".
  - A browser that cannot make a `Notification` in a page (Chrome on
    Android allows it only in a service worker) answers "unsupported".
  - The page has no image of the app's icon, so a notification has no icon
    (`AppFrame`'s `notifyIcon` option can give one).

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
| `appInfo(id)` | 1.9 | One app as JSON text, with the same fields as a `listApps` element. `null` if denied or if the app is not in the library. |
| `pin(id, version)` | 1.9 | Keeps the app on an installed version. An empty `version` removes the pin; then the newest version runs. |
| `blockPublisher(key)`, `unblockPublisher(key)` | 1.9 | Block or unblock a publisher key (the `signer` of an app). No app that the key signed opens or installs. |
| `checkUpdates(id, done)` | 1.9 | Checks the source of the app for a newer version, off the UI thread. An empty `id` checks every app. `done` gets `{ updates, errors }` as JSON text, or `null` if denied. The host keeps the result, so `listApps` shows it. |
| `update(id, done)` | 1.9 | Installs the newest version from the source of the app, off the UI thread. `done` gets `null` on success (also when the app is up to date), or the error text. The new version opens only after the user decides its new capabilities. |
| `lastError()` | 1.7 | The reason the last synchronous call was denied, or `null`. |

From core 1.9, each element of `listApps` also has these fields:
`publisherBlocked`, `pinned` (`""` if there is no pin), `versions` (the
installed versions, newest first), `update` (the newer version that the
last check found, or `""`), and `updateCapabilities` (the capabilities
that the update adds).

`search`, `install`, `checkUpdates` and `update` use the same request id
and `completion` event as `plinth:dialog`. Without `done`, each returns a
`Promise<string | null>` of the same value, for `await`. The web host answers every
`plinth:hub` call with "unsupported" and does not trap (the web Hub is
phase H6).

## Uncaught errors

An exception that the app does not catch (SPEC.md §5.6) does not stop
the app. The guest sends the text, for example `Uncaught Error: no
network`, to the host through `error.report` (core 1.10; no capability).
A rejected promise that nothing awaits is reported the same way, as
`Uncaught (in promise) Error: …`, at the end of the event.
The desktop runner logs it as an error and keeps it
(`Guest::take_errors`); the desktop host shows the last one in a banner
with a "Dismiss" button (not app UI). The web host calls `reportError`, which is
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
| `plinth:files` | The promise rejects with `Error("denied:<reason>")`; a `done` callback gets the reason as `error`. |
| `plinth:notify` | `show`'s promise rejects with `Error("denied:<reason>")`; `cancel` does nothing. |

Each denied module exposes `lastError()` (`kv.lastError()`,
`clipboard`'s module-level `lastError()`), which returns one of:

- `"denied:undeclared"` — the manifest does not declare the capability.
- `"denied:refused"` — the user declined it.
- `"denied:unsupported"` — this host does not implement the capability
  (for example, every `plinth:hub` call on a host with no Hub backend).
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

A package with no capabilities (like `examples/counter`) prints
`declared: none` and `reachable: none`.

## Support on each host

This section tells how each host API works on each host, and which test
covers it. There are three hosts:

- **Desktop (wasmtime):** the default engine of `plinth dev`, `plinth run`,
  `plinth native` and the desktop Hub.
- **Desktop (wasmi):** the interpreter for iOS. Set `PLINTH_ENGINE=wasmi`
  to use it on the desktop (cargo feature `wasmi` on
  `plinth-runner-wasmtime`). Each import shim in `wasmi_engine.rs` calls the
  same `HostState` method as wasmtime. Thus the behavior is the same as on
  wasmtime. Only the call time limit is different: wasmi uses fuel, not
  epochs (`docs/GAPS.md` "wasmi engine").
- **Web:** `web/plinth-web.js`. "In a frame" means the sandboxed app frame
  (the web Hub, a web export with `<plinth-app>`): the page does the work
  for the frame (`web/frame-host.js`). "Stand-alone" means
  `web/index.html`, for tests and development.

### Behavior

| API | Desktop (wasmtime) | Desktop (wasmi) | Web |
|---|---|---|---|
| `time`: timers | The host wakes at the next timer deadline, and at least every 15 ms. A late repeating timer fires one time, then keeps its period. | Same as wasmtime. | JS `setTimeout` and `setInterval`. A browser slows the timers of a hidden tab. |
| `time`: dates | The time zone offset comes from the OS. | Same as wasmtime. | The offset is `-Date.getTimezoneOffset()`. |
| `time`: `onFrame` | The host fires frame timers in the gpui frame. While a frame timer runs, the host asks for the next frame. | Same as wasmtime. | One `requestAnimationFrame` loop for all frame timers. The loop stops when the last frame timer stops. The browser sends no frames in a hidden tab. |
| `store` (kv) | One JSON file for each app id, `%APPDATA%\plinth\apps\<app id>\kv.json`. Each write writes the file again. No quota. A failed write answers `denied:unsupported`. If the host cannot open the file, the app gets an empty store in memory for this run (the log has a warning). | Same as wasmtime. | In a frame: the page `localStorage`, keys `plinth-hub:kv:<app id>:<key>`. The frame gets a snapshot at start and sends each write to the page. Quota: 512 Ki characters of keys and values for each app; a `set` over the quota answers `denied:refused`. Stand-alone: `localStorage`, keys `plinth:<app id>:<key>`. Node: a `Map` in memory. |
| `clipboard`: write | The system clipboard (gpui). | Same as wasmtime. | `navigator.clipboard.writeText`, in a frame through the page. The call does not wait for the browser. |
| `clipboard`: read | The system clipboard, at once. | Same as wasmtime. | **Different.** The browser API is asynchronous and the host call is not. `readText()` returns the last value that the host knows (or `null`), and starts a read for the next call. Thus the first read can return `null`. The browser can ask the user for permission. The browser gives the clipboard only to a secure page (HTTPS or `localhost`). |
| `dialog` | A modal overlay that the desktop host draws, one request at a time. Escape cancels; Enter confirms. | Same as wasmtime. | Stand-alone: the browser's `alert`, `confirm` and `prompt`. In a frame: a dialog that the page draws (`hostDialog`), because a sandboxed frame cannot show a browser dialog. In Node: a declined answer. |
| `net` | `ureq` on a worker thread. Time limit 20 s. Body limit 8 MiB (`MAX_BODY`). No redirects. The private-address check runs on the host text and again on the addresses after DNS resolution. | Same as wasmtime. | **Different.** `fetch` with a 20 s time limit, `redirect: "manual"` and `credentials: "omit"`. The host code has no body limit. The private-address check runs only on the host text (`docs/THREAT-MODEL.md` §6 item 1). In a frame, the page makes the request, so the request has the origin of the page. The server must allow CORS. |
| `files` | A folder for each owner and app id under `%APPDATA%\plinth\spaces\private\`. One worker thread for each app, in call order. 8 MiB for one file, 50 MiB for the space. | Same as wasmtime. | In a frame: the page IndexedDB (database `plinth-files`). Stand-alone and in Node: in memory, so the files go away at a reload. The same path rules and limits as the desktop (`web/files.js`). |
| `notify` | Windows: a WinRT toast; the tag replaces and `cancel` removes. Linux and macOS: `show` answers "unsupported". | Same as wasmtime. | **Different.** The browser's `Notification` API, shown by the page for the app frame. The tag replaces, `cancel` closes. A browser-denied permission answers `denied:refused`; no permission question or no API answers "unsupported". |
| `hub` | Only for a package that a trusted Hub key signed, with a Hub backend. Else each call answers `denied:unsupported` (or `denied:refused`). | Same as wasmtime. | Only in the web App Hub (`hub-host.js`, browse mode, `docs/web-hub.md` §3.1). Else each call answers `denied:unsupported`. |
| `error.report` | The runner logs the text and keeps it (`Guest::take_errors`). The desktop host shows the last one in a banner with "Dismiss". | Same as wasmtime. | `reportError`, which is `console.error` by default, and `PlinthApp.onError`. |

### Tests

"By hand" means `PLINTH_ENGINE=wasmi cargo test -p plinth-compiler --test
<name>`. The gate (`scripts/ci-local.sh`) does not run the wasmi engine
(`docs/PRODUCTION.md` Q4). The results of the last run by hand are in
`docs/GAPS.md` "wasmi engine". Compiler tests are in
`crates/plinth-compiler/tests/`; web tests are in `web/test/`. The gate
runs every web test except `run-a11y.mjs` (by hand).

| API | Desktop (wasmtime) | Desktop (wasmi) | Web |
|---|---|---|---|
| `time`: timers | `hostapi.rs` (`timer_and_kv_round_trip`), `sevenguis.rs` (`timer_elapses_to_the_duration_and_resets`), unit tests in `plinth-runner-wasmtime/src/timers.rs` | `sevenguis.rs`, by hand | `run-timer.mjs`, `run-7guis.mjs` |
| `time`: dates | `hostapi.rs` (`date_time_round_trip_with_a_fake_offset`), `sevenguis.rs` (`flight_booker_constraints`) | `sevenguis.rs`, by hand | `run-7guis.mjs` (flight booker) |
| `time`: `onFrame` | `apps.rs` (`on_frame_gets_the_frame_time`, `pong_plays_a_point`), `timers.rs` (`frame_timers_fire_only_by_frames`) | `apps.rs` (`pong_plays_a_point`), by hand | `run-pong.mjs` |
| `store` (kv) | `hostapi.rs` (`timer_and_kv_round_trip`, `kv_denied_by_policy_does_not_trap`), `capabilities.rs`, unit tests in `kv.rs` | `apps.rs` (`budget_add_filter_edit_delete_and_stats`), by hand | `run-notes.mjs`, `run-hub-storage.mjs` (namespaces, quota), `run-a11y.mjs` (kv isolation in the frame) |
| `clipboard`: write | **no test** | **no test** | **no test** (`run-hub-storage.mjs` checks only the shape of the bridge message) |
| `clipboard`: read | **no test** | **no test** | **no test** |
| `dialog` | `async_await.rs` (`the_dialogs_example_greets_with_three_awaits` and other tests) | **no test** | **no test** (`run-hub-storage.mjs` checks only the shape of the bridge message) |
| `net` | `net.rs` (local server: GET, POST, status, denied, private addresses, DNS), unit tests in `net.rs` and `policy.rs` | **no test** | `run-net.mjs`, `run-net-private.mjs`, `run-hub-storage.mjs` (the check of the page) |
| `files` | `files.rs`, unit tests in `plinth-runner-wasmtime/src/files.rs` | **no test** | `run-files.mjs`, `run-a11y.mjs` (files isolation in the frame) |
| `notify` | `notify.rs` (a hand-written core caller and compiled apps, with `MemoryNotify`), unit tests in `plinth-runner-wasmtime/src/notify.rs` and `plinth-host-desktop/src/notify.rs` (the real toast only by hand: `--ignored`) | `notify.rs`, by hand | `run-notify.mjs` (a fake `Notification`, `notify-demo`), `run-a11y.mjs` (`notify-demo` export in Edge, without and with the permission) |
| `hub` | `hub_api.rs`, `hub_ui.rs`, `plinth-host-desktop/tests/hub_host.rs` | **no test** | `run-hub-host.mjs`, `run-a11y.mjs` (the Hub app in the browser) |
| `error.report` | `errors.rs`, `async_await.rs` (uncaught rejections) | `errors.rs`, by hand | `run-stop.mjs` |

`monkey.rs` sends random events to every example, also `utility`, which
calls `plinth:clipboard`. It does not check the clipboard, so this table
does not count it as a clipboard test.
