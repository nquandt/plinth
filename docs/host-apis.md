# Host APIs, capabilities, and the manifest

A Plinth app cannot reach the outside world by itself. Every effect
outside its own UI — storage, the clipboard, the network, even a modal
dialog — goes through a **host API module** (`plinth:time`,
`plinth:store`, `plinth:clipboard`, `plinth:dialog`, `plinth:net`,
`plinth:files`, `plinth:notify`, `plinth:secrets` and `plinth:locale`). Most of these modules are gated by a **capability**:
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
| `formatDate(ms, pattern, utc?)` | A small pattern language: `YYYY MM M DD D HH H hh h mm ss SSS ddd dddd MMM MMMM A`; any other character passes through. English names only — locale-aware formatting is `formatDate(ms, { dateStyle, timeStyle })` of `plinth:locale` (core 1.13). |
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

**Quota.** Each app has a data quota (100 MiB by default; `docs/STORAGE.md`
§7.3). A `kv.set` that would make the keys and values bigger than the quota
does nothing, and `kv.lastError()` returns `"denied:refused"`. A `set` that
does not make the store bigger works. The web host answers the same way over
its own quota.

### `sql` (core 1.13)

A SQLite database for each app (`docs/SQL.md`). Needs the `store.sql`
capability. Its risk is Low: the database belongs to this app only, so the
host grants it with no question. Every call returns a `Promise`.

```ts
import { sql } from "plinth:store";

type Note = { id: number; title: string; done: boolean };

await sql.migrate([
  "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0)",
]);                                                       // the new version: 1
const r = await sql.exec("INSERT INTO notes (title) VALUES (?)", ["Buy milk"]);
// r.changes = 1, r.lastInsertId = 1
const open = await sql.query<Note>("SELECT id, title, done FROM notes WHERE done = ?", [false]);
await sql.batch([
  { sql: "UPDATE notes SET done = 1 WHERE id = ?", params: [1] },
  { sql: "DELETE FROM notes WHERE done = 1" },
]);                                                       // all steps or none
```

- A parameter is a `number`, a `string`, a `boolean` (stored as 0 or 1) or
  `null`. An array literal can mix them; an array value must have one of
  these element types.
- `query<T>` decodes each row into `T` as `JSON.parse<T>` decodes a value,
  one field for each column. A `boolean` field also reads 0 and 1. A
  missing column, or a column of the wrong type, rejects the promise with
  `Error("sql: a row does not match the type ...")`.
- `migrate(steps)` runs the steps after the database's `user_version` in
  one transaction and resolves with the new version (the number of steps).
  Steps are append-only.
- A SQL error rejects with `Error("sql: <message>")`, for example
  `"sql: the query took too long"` (2 s), `"sql: the database is full"`
  (the data quota: 100 MiB on the desktop by default, 256 MiB on the web) or `"sql: not authorized"` (`ATTACH`, `BEGIN`, `VACUUM` and
  most `PRAGMA`s). Use `batch` for a transaction.
- The compiler gives warning `PL2026` when a statement is not a string
  literal or a `const`. Give values as `?` parameters: a statement built
  from strings can let SQL injection in. The compiler cannot see the
  statements of a `batch` or `migrate` array value, so it does not check
  them.

On the desktop host, the database is `store.sqlite` in the folder
`<private space folder>+sql`, next to the `plinth:files` space and not in
it. On the web host, the page runs SQLite (the official Wasm build) in its
own Worker with OPFS, one database for each app id; without OPFS the
database is in memory. The folder form of a web export has `sql` (the
export writes the SQLite files next to `plinth.js`). A single-file web
export does not: each call rejects with `Error("denied:unsupported")`. `docs/SQL.md` §9 lists the
differences of the web engine.

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

**Limits.** 8 MiB for one file; the data quota for the private space of one
app (the sum of the file sizes): 100 MiB on the desktop by default (a host
setting, `docs/STORAGE.md` §7.3), 50 MiB on the web. A `write` over the quota
rejects with `Error("quota")`.

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

## `plinth:secrets`

Secure storage for an app's own passwords and tokens (core 1.13, `docs/SECRETS.md`). Needs the `secrets` capability. Its risk is Low: the store belongs to this app only, like `kv`, so the host grants it with no question.

```ts
import { set, get, remove, names, lastError } from "plinth:secrets";

await set("api-token", "abc123");   // Promise<void>
const t = await get("api-token");   // Promise<string | null>
await remove("api-token");
const all = await names();          // Promise<string[]>, sorted
```

- A name has 1 to 64 characters from `a-z A-Z 0-9 . _ -`. A value is a string of at most 16 KiB. An app has at most 64 secrets.
- `get` resolves with `null` when the name does not exist.
- A denied call rejects with `Error("denied:<reason>")`. The reasons are `undeclared`, `refused`, `unsupported`, `limit` and `failed`. `unsupported` means this host has no safe store. The host never falls back to a plain file.
- The host never logs a value. Secrets stay on the device and are not in the sync of app data.

**Where each host keeps them.**

- Windows: Credential Manager (`plinth-host-desktop/src/secrets.rs`, the `keyring` crate). The target is `plinth/<app id>/default/<name>`. A blob holds at most 2560 bytes, so the host cuts a value into chunks of 2 KiB.
- macOS, Linux: the native store through `keyring`. Not tested yet.
- Web: a WebCrypto AES-GCM key (not extractable) in IndexedDB encrypts the values (`web/secrets.js`). This protects against a copy of the browser profile on disk. It does not protect against code that runs in the page. The page does the work for the sandboxed frame.
- iOS: the Keychain. Not built yet (`docs/MAC-HANDOFF.md` §5).

## `plinth:locale`

The locale of the app (core 1.13, `docs/I18N.md` §3.1, §4 step 1). Needs
no capability, as for the time zone.

```ts
import { locale, direction } from "plinth:locale";

<Text>{`${locale()} ${direction()}`}</Text>   // "ar-EG rtl"
```

- `locale()` returns the locale as a BCP 47 tag, for example `"en-US"`,
  `"ar-EG"` or `"zh-Hans-CN"`. It is never empty: a host that finds no
  locale gives `"en-US"`.
- `direction()` returns `"rtl"` when the script of the locale is written
  right to left, else `"ltr"`. The type is `Direction` (`"ltr" | "rtl"`).
  A script subtag decides (`az-Arab` is `rtl`, `az-Latn` is `ltr`); with
  no script subtag, the language decides (`ar`, `arc`, `azb`, `ckb`, `dv`,
  `fa`, `he`, `iw`, `ji`, `ks`, `lrc`, `mzn`, `nqo`, `pnb`, `ps`, `sd`,
  `syr`, `ug`, `ur` and `yi` are `rtl`).
- Both are reactive, as `isActive()`: a text, a `computed` value or an
  `effect` that reads them runs again when the locale changes while the app
  runs. A value that the app keeps in a plain variable does not change.
- The host normalizes the tag: `_` becomes `-`, `.UTF-8` and `@euro` go
  away, the language is lower case, a 4-letter script is title case and a
  region is upper case (`en_us.UTF-8` gives `en-US`).
- The host does not yet match the locale against the languages that the
  app supports (`docs/I18N.md` §3.1 and §4 step 3).

**The change event.** The host sends a `lifecycle` event with kind 4
("locale changed", `plinth_protocol::lifecycle_kind::LOCALE`). The core
then makes the reactive reads run again; they call `locale.tag` and
`locale.is-rtl` again. A core older than 1.13 ignores kind 4, so a host can
send it to every app.

**Where the locale comes from.**

- Desktop: the OS locale (`sys-locale`: Windows `GetUserDefaultLocaleName`,
  macOS `CFLocale`, Linux `LANG`), read at the first call. The
  environment variable `PLINTH_LOCALE` (for example `PLINTH_LOCALE=ar-EG`)
  overrides it, for manual tests. The host reads the OS locale again each
  time the app window comes to the foreground, and sends the event if it
  changed. A test fixes the locale with `Guest::set_locale` and changes it
  with `Guest::change_locale`.
- Web: in a frame (the web Hub, a web export), the page gives its locale
  (`navigator.languages[0]`) in the `start` message, and sends each
  `languagechange` of the page to the frame (`locale`). `AppFrame`'s
  `locale` option fixes the locale; `setLocale(tag)` changes it. The
  stand-alone page uses its own locale, and `index.html?locale=ar-EG` fixes
  it. The host sets `lang` and `dir` on the root element of the app
  (`applyLocale` in `web/dom-renderer.js`).

**Formatting** (core 1.13, `docs/I18N.md` §3.7, §4 step 2). The host formats
in the locale of the app: ICU4X on the desktop, the browser's `Intl` on the
web. The calls are synchronous and reactive, as `locale()`. Write the
options as an object literal at the call.

```ts
import { formatNumber, formatDate, pluralCategory, formatList, compare } from "plinth:locale";

formatNumber(1234.5)                                        // "1,234.5" (en-US), "1.234,5" (de-DE)
formatNumber(0.256, { style: "percent" })                   // "26%"
formatNumber(1234.5, { style: "currency", currency: "EUR" }) // "€1,234.50", "1.234,50 €"
formatNumber(1234567, { notation: "compact" })              // "1.2M"
formatDate(now(), { dateStyle: "medium", timeStyle: "short" }) // "Oct 8, 2026, 11:44 AM"
pluralCategory(3)                                           // "other" (en), "few" (pl, ru)
pluralCategory(2, "ordinal")                                // "two" (en: "2nd")
formatList(["a", "b", "c"])                                 // "a, b, and c"
names.sort((a, b) => compare(a, b))                         // "adam, bob, Émile, Zoë"
```

| Function | Options | Notes |
|---|---|---|
| `formatNumber(value, options?)` | `style` ("decimal", "percent", "currency"), `currency` (ISO 4217), `minimumFractionDigits`, `maximumFractionDigits` (0 to 20), `useGrouping`, `notation` ("standard", "compact") | Defaults: 0 to 3 fraction digits ("decimal"), 0 ("percent"); the currency sets its digits. The digit options and `useGrouping` do not apply to "currency" and "compact". A bad currency code gives "decimal". A value that is not finite gives "". |
| `formatDate(ms, options?)` | `dateStyle` ("full", "long", "medium", "short"), `timeStyle` ("medium", "short") | The local time zone of the host. No options: `dateStyle` "medium". Both hosts give a space (U+0020) where CLDR has U+202F. |
| `pluralCategory(value, type?)` | `type`: "cardinal" (default) or "ordinal" | Returns `PluralCategory`: "zero", "one", "two", "few", "many" or "other". |
| `formatList(items, type?)` | `type`: "conjunction" (default) or "disjunction" | The wide form. |
| `compare(a, b)` | none | The collation of the locale: -1, 0 or 1. |

`formatDate` of `plinth:time` stays: it formats with a pattern and English
names. To use both in one file, import one with another name.

The two hosts give the same text, except for the known differences in
`docs/I18N.md` §3.7 (some currency symbols in fr-FR and pl-PL, the percent
sign in ar-EG, the short date in pl-PL). The shared test cases are in
`crates/plinth-compiler/tests/locale_cases.txt`.

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
| `uninstall(id, keepData, done)` | 1.13 | Removes the app from the library and, unless `keepData` is `true`, deletes its data, its grants and its secrets (`docs/STORAGE.md` §7), off the UI thread. `done` gets `null` on success, or the error text. The error text says why nothing changed: the app runs (the user must close its windows), the app is not in the library, or the app is the Hub app itself (the Hub cannot uninstall itself). `denied:<reason>` if the call is denied. |
| `lastError()` | 1.7 | The reason the last synchronous call was denied, or `null`. |

From core 1.9, each element of `listApps` also has these fields:
`publisherBlocked`, `pinned` (`""` if there is no pin), `versions` (the
installed versions, newest first), `update` (the newer version that the
last check found, or `""`), and `updateCapabilities` (the capabilities
that the update adds).

`search`, `install`, `checkUpdates`, `update` and `uninstall` use the same
request id and `completion` event as `plinth:dialog`. Without `done`, each
returns a `Promise<string | null>` of the same value, for `await`.

`remove(id)` hides the app and keeps its data. `uninstall(id, keepData)`
deletes the app and, unless `keepData`, its data (`docs/STORAGE.md` §7.2).
The command `plinth hub uninstall` does the same. A host with no Hub backend
answers every `plinth:hub` call with "unsupported" and does not trap. The
web Hub gives the Hub app a backend (`docs/web-hub.md`).

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
| `plinth:store` `sql` | The promise rejects with `Error("denied:<reason>")`. |
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
| `store` (kv) | One JSON file for each app id, `%APPDATA%\plinth\apps\<app id>\kv.json`. Each write writes the file again. Quota: the data quota (100 MiB by default); a `set` over it answers `denied:refused`. A failed write answers `denied:unsupported`. If the host cannot open the file, the app gets an empty store in memory for this run (the log has a warning). | Same as wasmtime. | In a frame: the page `localStorage`, keys `plinth-hub:kv:<app id>:<key>`. The frame gets a snapshot at start and sends each write to the page. Quota: 512 Ki characters of keys and values for each app; a `set` over the quota answers `denied:refused`. Stand-alone: `localStorage`, keys `plinth:<app id>:<key>`. Node: a `Map` in memory. |
| `clipboard`: write | The system clipboard (gpui). | Same as wasmtime. | `navigator.clipboard.writeText`, in a frame through the page. The call does not wait for the browser. |
| `clipboard`: read | The system clipboard, at once. | Same as wasmtime. | **Different.** The browser API is asynchronous and the host call is not. `readText()` returns the last value that the host knows (or `null`), and starts a read for the next call. Thus the first read can return `null`. The browser can ask the user for permission. The browser gives the clipboard only to a secure page (HTTPS or `localhost`). |
| `dialog` | A modal overlay that the desktop host draws, one request at a time. Escape cancels; Enter confirms. | Same as wasmtime. | Stand-alone: the browser's `alert`, `confirm` and `prompt`. In a frame: a dialog that the page draws (`hostDialog`), because a sandboxed frame cannot show a browser dialog. In Node: a declined answer. |
| `net` | `ureq` on a worker thread. Time limit 20 s. Body limit 8 MiB (`MAX_BODY`). No redirects. The private-address check runs on the host text and again on the addresses after DNS resolution. | Same as wasmtime. | **Different.** `fetch` with a 20 s time limit, `redirect: "manual"` and `credentials: "omit"`. The host code has no body limit. The private-address check runs only on the host text (`docs/THREAT-MODEL.md` §6 item 1). In a frame, the page makes the request, so the request has the origin of the page. The server must allow CORS. |
| `files` | A folder for each owner and app id under `%APPDATA%\plinth\spaces\private\`. One worker thread for each app, in call order. 8 MiB for one file, the data quota (100 MiB by default) for the space. | Same as wasmtime. | In a frame: the page IndexedDB (database `plinth-files`). Stand-alone and in Node: in memory, so the files go away at a reload. The same path rules and limits as the desktop (`web/files.js`). |
| `store` (sql) | SQLite (`rusqlite`, bundled, with FTS5): `store.sqlite` in `<private space folder>+sql`. One worker thread for each app, in call order. 2 s for one statement, 256 MiB for the database, 16 MiB for a query result. | Same as wasmtime. | **Different.** The official SQLite Wasm build in one Worker of the page for all apps (`web/sql.js`), with the OPFS VFS `opfs-sahpool` (folder `.plinth-sql`, one database for each app id in lower case). The same authorizer, limits and error texts; the memory limit is the Wasm memory of the Worker (512 MiB). Without OPFS: in memory. The folder form of a web export has it; a single-file export answers `denied:unsupported`. `docs/SQL.md` §9. |
| `notify` | Windows: a WinRT toast; the tag replaces and `cancel` removes. Linux and macOS: `show` answers "unsupported". | Same as wasmtime. | **Different.** The browser's `Notification` API, shown by the page for the app frame. The tag replaces, `cancel` closes. A browser-denied permission answers `denied:refused`; no permission question or no API answers "unsupported". |
| `locale` | The OS locale (`sys-locale`), or `PLINTH_LOCALE`. The host reads it again when the window comes to the foreground, and sends the locale event if it changed. | Same as wasmtime. | In a frame: the locale of the page (`navigator.languages`), from the `start` message, and the page's `languagechange`. Stand-alone: the page's own locale, or `?locale=`. The host sets `lang` and `dir` on the root element of the app. |
| `app-info` (`onUpgrade`) | A file `data-version` next to `kv.json` (`%APPDATA%\plinth\apps\<app id>\`). | Same as wasmtime. | In a frame: the page `localStorage`, key `plinth-hub:dv:<app id>`, sent in the `start` message; the Hub removes it with the app's kv data. Stand-alone: `localStorage`, key `plinth-data-version:<app id>`. Node: a `Map` in memory. |
| `hub` | Only for a package that a trusted Hub key signed, with a Hub backend. Else each call answers `denied:unsupported` (or `denied:refused`). | Same as wasmtime. | Only in the web App Hub (`hub-host.js`, browse mode, `docs/web-hub.md` §3.1). Else each call answers `denied:unsupported`. |
| `error.report` | The runner logs the text and keeps it (`Guest::take_errors`). The desktop host shows the last one in a banner with "Dismiss". | Same as wasmtime. | `reportError`, which is `console.error` by default, and `PlinthApp.onError`. |

### Tests

"wasmi gate" means `bash scripts/ci-local.sh wasmi`, which runs the test
with `PLINTH_ENGINE=wasmi` (also the CI job "Windows (wasmi engine)",
`docs/PRODUCTION.md` Q4). "By hand" means `PLINTH_ENGINE=wasmi cargo test
-p plinth-compiler --test <name>`, for a suite that is too slow for the
gate in a debug build. The results are in `docs/GAPS.md` "wasmi engine". Compiler tests are in
`crates/plinth-compiler/tests/`; web tests are in `web/test/`. The gate
runs every web test except `run-a11y.mjs` (by hand).

| API | Desktop (wasmtime) | Desktop (wasmi) | Web |
|---|---|---|---|
| `time`: timers | `hostapi.rs` (`timer_and_kv_round_trip`), `sevenguis.rs` (`timer_elapses_to_the_duration_and_resets`), unit tests in `plinth-runner-wasmtime/src/timers.rs` | `hostapi.rs`, wasmi gate; `sevenguis.rs`, by hand | `run-timer.mjs`, `run-7guis.mjs` |
| `time`: dates | `hostapi.rs` (`date_time_round_trip_with_a_fake_offset`), `sevenguis.rs` (`flight_booker_constraints`) | `hostapi.rs`, wasmi gate; `sevenguis.rs`, by hand | `run-7guis.mjs` (flight booker) |
| `time`: `onFrame` | `apps.rs` (`on_frame_gets_the_frame_time`, `pong_plays_a_point`), `timers.rs` (`frame_timers_fire_only_by_frames`) | `apps.rs` (`pong_plays_a_point`), wasmi gate | `run-pong.mjs` |
| `store` (kv) | `hostapi.rs` (`timer_and_kv_round_trip`, `kv_denied_by_policy_does_not_trap`), `capabilities.rs`, unit tests in `kv.rs` | `hostapi.rs`, `capabilities.rs`, `apps.rs` (`budget_add_filter_edit_delete_and_stats`), wasmi gate | `run-notes.mjs`, `run-hub-storage.mjs` (namespaces, quota), `run-a11y.mjs` (kv isolation in the frame) |
| `app-info` (`onUpgrade`) | `upgrade.rs`, unit tests in `app_info.rs` | `upgrade.rs`, wasmi gate | `run-upgrade.mjs` (also `WorkerApp` across a reload, the Hub storage) |
| `clipboard`: write | `clipboard.rs` (`clipboard_write_then_read_round_trips_unicode`, `clipboard_denied_by_policy_does_not_trap`) | `clipboard.rs`, wasmi gate | `run-clipboard-dialog.mjs` (`run-hub-storage.mjs` also checks the shape of the bridge message) |
| `clipboard`: read | `clipboard.rs` (`clipboard_write_then_read_round_trips_unicode`, `clipboard_read_of_an_empty_clipboard_is_null`, `clipboard_denied_by_policy_does_not_trap`) | `clipboard.rs`, wasmi gate | `run-clipboard-dialog.mjs` |
| `dialog` | `async_await.rs` (`the_dialogs_example_greets_with_three_awaits` and other tests) | `async_await.rs`, wasmi gate | `run-clipboard-dialog.mjs` (`alert`, `confirm`, `prompt`; `run-hub-storage.mjs` also checks the shape of the bridge message) |
| `net` | `net.rs` (local server: GET, POST, status, denied, private addresses, DNS), unit tests in `net.rs` and `policy.rs` | `net.rs`, wasmi gate | `run-net.mjs`, `run-net-private.mjs`, `run-hub-storage.mjs` (the check of the page) |
| `files` | `files.rs`, unit tests in `plinth-runner-wasmtime/src/files.rs` | `files.rs`, wasmi gate | `run-files.mjs`, `run-a11y.mjs` (files isolation in the frame) |
| `store` (sql) | `sql.rs` (compiled apps: the shared cases of `sql_cases.txt`, the file on disk, denied, the warning), unit tests in `plinth-runner-wasmtime/src/sql.rs` | `sql.rs`, wasmi gate | `run-sql.mjs` (the engine with the cases of the desktop unit tests, the shared cases of `sql_cases.txt` built by the CLI, the Worker protocol, denied calls), `run-a11y.mjs` (a fixture app in the web App Hub in Edge: OPFS after a reload) |
| `notify` | `notify.rs` (a hand-written core caller and compiled apps, with `MemoryNotify`), unit tests in `plinth-runner-wasmtime/src/notify.rs` and `plinth-host-desktop/src/notify.rs` (the real toast only by hand: `--ignored`) | `notify.rs`, wasmi gate | `run-notify.mjs` (a fake `Notification`, `notify-demo`), `run-a11y.mjs` (`notify-demo` export in Edge, without and with the permission) |
| `locale` | `locale.rs` (compiled apps with five fixed locales, the change event, the normalization, the core version), unit tests in `plinth-runner-wasmtime/src/locale.rs` | `locale.rs`, wasmi gate | `run-locale.mjs` (the tag and the direction, `PlinthApp` and the Worker, the change event, `lang` and `dir`, `AppFrame` and `languagechange`) |
| `hub` | `hub_api.rs`, `hub_ui.rs`, `plinth-host-desktop/tests/hub_host.rs` | `hub_api.rs`, `hub_ui.rs`, wasmi gate | `run-hub-host.mjs`, `run-a11y.mjs` (the Hub app in the browser) |
| `error.report` | `errors.rs`, `async_await.rs` (uncaught rejections) | `errors.rs`, `async_await.rs`, wasmi gate | `run-stop.mjs` |

`monkey.rs` sends random events to every example, also `utility`, which
calls `plinth:clipboard`. It does not check the clipboard, so this table
does not count it as a clipboard test.
