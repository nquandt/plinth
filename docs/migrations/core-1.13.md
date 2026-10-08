# Core 1.13

Status: not shipped yet. Core 1.12 is the newest shipped core (`rt_abi::SHIPPED_MINOR`). Items can still be added to 1.13 until it ships.

Core 1.13 adds the SQL store (`store.sql`, `docs/SQL.md`), a hash table for `Map` and `Set`, and the module `plinth:locale` (`docs/I18N.md` §4 steps 1 and 2). UI API 1.13, which comes with it, adds context menus, drag and drop in one app, and a form in a `Dialog`. Nothing is removed or changed. Every app that runs on core 1.12 runs on core 1.13 with the same behavior.

## 1. What is new

| Item | Kind | Notes |
|---|---|---|
| `sql_call` | Core function | Starts one `store.sql` call. The result comes back through the same completion path as `plinth:files` (`net_result_*`). |
| `plinth:app/sql` with `run(kind, statement, params) -> u32` | Host interface (`wit/plinth/app.wit`) | Kind 0 is `exec`, 1 is `query`, 2 is `batch`, 3 is `migrate`. `params` and the result are JSON text. |
| `sql` in `plinth:store` | Plinth TS module export (`std/store.d.ts`) | `sql.exec`, `sql.query<T>`, `sql.batch`, `sql.migrate`. |
| `store.sql` | Capability, risk Low | Declare it in `plinth.toml` with a rationale. |
| `PL2026` | Compiler warning | A SQL statement that is not a string literal or a `const`. |
| `kv_find_str`, `kv_find_f64`, `kv_find_i32`, `kv_add`, `kv_remove`, `kv_clear`, `kv_size`, `kv_live`, `kv_live_copy`, `kv_iter` | Core functions (`crates/plinth-rt/src/kv.rs`) | A hash table for `Map` and `Set`. Before, each `get`, `set`, `has` and `delete` searched the keys one by one: 10,000 string keys took 1,558 ms, and now take 13 ms. A loop over a map visits each entry one time, also when the loop body adds or deletes keys. |
| `locale_tag`, `locale_direction` | Core functions | `locale()` and `direction()` of `plinth:locale`. Each read is reactive: the locale event makes it run again. |
| `plinth:app/locale` with `tag() -> string` and `is-rtl() -> bool` | Host interface (`wit/plinth/app.wit`) | Synchronous. No capability. `tag` is a BCP 47 tag and is never empty. |
| `lifecycle` event, kind 4 (locale changed) | Event (SPEC §8.4, `plinth_protocol::lifecycle_kind::LOCALE`) | No payload. The core reads the locale again. A core older than 1.13 ignores it. |
| `locale`, `direction` and the type `Direction` in `plinth:locale` | Plinth TS module (`std/locale.d.ts`) | `locale(): string`, `direction(): "ltr" \| "rtl"`. |
| `locale_format_number`, `locale_format_date`, `locale_plural`, `locale_format_list`, `locale_compare` | Core functions | The formatting functions of `plinth:locale` (`docs/I18N.md` §3.7). Each call is reactive, as `locale()`. |
| `plinth:app/locale` `format-number(value: f64, options: string) -> string`, `format-date(ms: f64, options: string) -> string`, `plural-category(value: f64, kind: string) -> string`, `format-list(items: string, kind: string) -> string`, `compare(a: string, b: string) -> s32` | Host interface (`wit/plinth/app.wit`) | Synchronous. No capability. `options` is a JSON object and `items` is a JSON array of strings. |
| `formatNumber`, `formatDate`, `pluralCategory`, `formatList`, `compare` and the types `NumberFormatOptions`, `DateFormatOptions`, `PluralCategory`, `PluralType`, `ListType` in `plinth:locale` | Plinth TS module (`std/locale.d.ts`) | Numbers, dates, plural categories, lists and collation in the locale of the app. |
| Merge sort for `Array.prototype.sort` | Compiler output | No core function. A sort of 10,000 numbers took 560 ms, and now takes 5 ms. The sort stays stable, with the same order. |
| `context-menu` event, id 12 | UI API 1.13 event (`wit/plinth/ui-api.toml`) | Value `[x, y]`, as the pointer events. `onContextMenu(x, y)` on `Box`, `Pressable`, `Scroll`, `Canvas` and `Row`. No core function: the core 1.12 functions `arg_at_f64` and `arg_at_i32` pass the two numbers. |
| `open` and `onClose` on `Menu` | UI API 1.13 props (`value`, id 4, and the event `close`, id 6) | No new id. A `Menu` with `value` has no button; the host shows it at the point of the last context-menu event. |
| `drag-value` prop, id 80 | UI API 1.13 prop (`wit/plinth/ui-api.toml`) | A string. `dragValue` on `Box`, `Pressable` and `Row`: the user can drag the element. |
| `drop` event, id 13 | UI API 1.13 event | Value `[drag value, x, y]` (`x` and `y` in the units of the pointer events). `onDrop(value, x, y)` on `Box`, `Pressable`, `Scroll`, `Canvas` and `Row`. No core function: the core 1.12 functions `arg_i32` (the string) and `arg_at_f64` pass the three items. |
| `reorder` event, id 14 | UI API 1.13 event | Value `[from, to]` (two ints, indexes of the rows). `onReorder(from, to)` on `List`. No core function. |
| Children and `onClose` on `Dialog` | UI API 1.13 (the event `close`, id 6) | No new id. The children that are not `Action`s are a small form between the message and the actions. |

## 2. App authors

You do not have to change an app.

- **An app that uses neither `sql` nor `Map` nor `Set`** still declares core 1.12 as the lowest core it needs (`plinth-core` section, SPEC §10.5). It keeps running on hosts that have only core 1.12.
- **An app that uses `Map` or `Set`** declares core 1.13 when you build it with the new compiler, because the new `Map` code uses the 1.13 functions. The same app, built with an older compiler, still declares the older core and keeps running.
- **An app that uses `plinth:locale`** declares core 1.13. An app that does not import it does not need core 1.13 because of it.
- **An app that uses `sql`** declares core 1.13. A host that has only older cores cannot run these apps. The host tells the user which core to install. Before you publish such an app, make sure that your users have a host with core 1.13.
- **The `core` policy** (`plinth.toml`, SPEC §10.5):
  - `follow` (the default): the app gets core 1.13 when the host has it. No action.
  - `patch`: the app stays on the minor version it was built with (`core-built`). Build it again with the new CLI to move it to 1.13.
  - `pinned`: the app stays on the exact core build. Build it again to move it.

To use the SQL store:

1. Add the capability to `plinth.toml`:

   ```toml
   [[capabilities]]
   name = "store.sql"
   rationale = "Save your notes on this device and search them."
   ```

2. Import `sql` from `plinth:store` and make the schema with `sql.migrate`. `docs/guides/storage.md` §3.1 has an example.
3. If the app keeps a list in `kv` today, move the data one time on the first run. `examples/notes/app/storage.ts` shows how: it reads the old `kv` value, writes the rows in one `sql.batch`, and then removes the `kv` key.
4. Keep `store.kv` in `plinth.toml` while the app moves old data. Its rationale tells the user why.

To use a context menu (UI API 1.13), give an element `onContextMenu` and open a `Menu` with `open` in the handler (`docs/ui.md` "Context menus"). It needs no core 1.13: an app that uses it still declares the core it declared before. A host with an older UI API never sends the event and shows the `Menu` as a button, so keep a second path to the same actions (a button or a keyboard key).

To let the user move the rows of a list (UI API 1.13), give the `List` `onReorder(from, to)` and move the item in your data (`docs/ui.md` "Drag and drop"; `examples/todo` `moveTask`). To drag an element onto a target, give the element `dragValue` and the target `onDrop(value, x, y)`. These need core 1.12 (callbacks with more than one argument), not core 1.13: an app that used no core 1.12 function now declares core 1.12 (`examples/todo` went from 1.0 to 1.12). A host with an older UI API shows no drag and sends no event, so keep a second path (buttons) where the move is necessary.

To put a small form in a `Dialog` (UI API 1.13), give it children (a `Slider`, a `TextField`) and `onClose` (`docs/ui.md` "Dialog with a form"). A host with an older UI API shows the dialog without its children. For such a host, a `Sheet` is the safe choice.

To use the locale, import `locale` and `direction` from `plinth:locale` (`docs/host-apis.md` "plinth:locale"). No capability is necessary. Read them in the UI, in a `computed` value or in an `effect`, so that the app follows a change of the locale.

To format in the locale, import `formatNumber`, `formatDate`, `pluralCategory`, `formatList` or `compare` from `plinth:locale`. Write the options as an object literal at the call. `formatDate` of `plinth:time` does not change: it keeps its pattern and its English names. A file that uses both imports one with another name.

## 3. Host authors

A host that uses core 1.13 must supply `plinth:app/sql` `run` and the seven functions of `plinth:app/locale`, or the core does not link (`docs/COMPATIBILITY.md` §3).

For `plinth:app/locale`:

- `tag` returns a BCP 47 tag, never an empty string (`en-US` when the host finds no locale). Normalize a POSIX name: `en_US.UTF-8` gives `en-US`.
- `is-rtl` follows the script subtag, else the language subtag, with the lists of `crates/plinth-runner-wasmtime/src/locale.rs` (the web host has the same lists in `web/plinth-web.js`).
- When the locale changes while the app runs, send a `lifecycle` event with kind 4. Send it only after the new value is ready for `tag`. An app with a core older than 1.13 ignores it.
- The web host sets `lang` and `dir` on the root element of the app.
- The formatting functions follow `docs/I18N.md` §3.7: the option sets, the defaults, the rules for input that is not valid (never trap, never throw), the local time zone for `format-date`, and a space (U+0020) in place of U+202F in a date. `crates/plinth-compiler/tests/locale_cases.txt` has the shared cases; the desktop host (ICU4X) and the web host (`Intl`) both pass them, with the known differences that the file records.

- A host with no SQL engine answers each call with `[false, 0, "", "denied:unsupported"]`. It must not leave the import out.
- A host with a SQL engine follows `docs/SQL.md` §4: the authorizer, the time limit, the quota, the result limit, and the error texts of `crates/plinth-runner-wasmtime/src/sql.rs`. `crates/plinth-compiler/tests/sql_cases.txt` has the shared cases; the desktop and the web host both pass them.
- The host checks the capability `store.sql` before the call, as for `plinth:files`.
- The hosts in this repo do all of this: the desktop host (wasmtime and wasmi), the web host, the web Hub and the folder form of a web export. A single-file web export answers `denied:unsupported`.

For UI API 1.13 (context menus), a host must:

- Send `context-menu` with `[x, y]` (the units of the pointer events) for a secondary click, for the menu key and for Shift+F10 on the focused element (`x` and `y` are its center), and on a touch screen for a long press when the platform has one. Send it only to the innermost element with a handler. Make an element with a handler a tab stop.
- Show a `Menu` that has a `value` prop with no button. When `value` becomes true right after a context-menu event, show it at the point of that event; else show it where it is in the layout. Give the focus to its first action. When the user closes it, set `value` to false in the host tree, then send `change` (false) and `close`. Give the focus back.
- Keep pointer input away from the elements under a modal layer (`Sheet`, `Dialog`, host dialogs).

For UI API 1.13 (drag and drop in one app, a form in a `Dialog`), a host must:

- Start a drag on an element with `drag-value`, or on a row (a direct child) of a `List` with a `reorder` listener: with the main button, after a move; with a finger, after a long press and a move (a move before the long press scrolls). Do not start a drag in a text field or on a slider, or under a modal layer.
- During a drag, show a drop target: an element with a `drop` listener (not the source) under the pointer, for a source with a `drag-value`; or another row of the same list, with a line on the side where the row goes. Only the innermost target counts.
- On the release over a target, send `drop` with `[drag value, x, y]` to the target, or `reorder` with `[from, to]` (row indexes) to the list. Do not change the order of the rows yourself.
- On Alt+Up and Alt+Down, while a row of such a list or a control in it has the focus, send `reorder` with `[i, i - 1]` or `[i, i + 1]` (nothing at the ends), and keep the focus on the row. Make a `Row` in such a list a tab stop.
- Tell assistive technology the result of a drag or a move (a polite live region).
- Show the children of a `Dialog` that are not `Action`s between the message and the actions. Give the focus to the first field when it opens. Do not close it on Enter in a field. When the user closes it, send `change` (false) and `close`, as for a `Sheet`.

The desktop host (`crates/plinth-ui/src/render.rs`) and the web host (`web/dom-renderer.js`) do all of this.

## 4. Users

Nothing changes for users. An app that uses `store.sql` shows the capability in its details with its rationale. Its risk is Low, so the Hub does not ask at install.
