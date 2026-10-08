# Guide: save data

This guide tells you how to save data in a Plinth app. It is for app authors. The reference is [host-apis.md](../host-apis.md) (`plinth:store`, `plinth:files`) and [SQL.md](../SQL.md) (`sql`).

## 1. Choose the API

Plinth gives an app three places for its data. All are private to the app: no other app can read them.

| API | Use it for | Capability | Calls |
|---|---|---|---|
| `plinth:store` (`kv`) | Small values: settings, a list of short items, the last screen. | `store.kv` | Synchronous. A value is a string. |
| `plinth:store` (`sql`) | Records that you filter, sort, count or search: notes, tasks, contacts, a history. | `store.sql` | Asynchronous. Each call returns a `Promise`. One SQLite database for each app. |
| `plinth:files` | Larger text: documents, exports. | `files.private` | Asynchronous. Each call returns a `Promise`. A file is UTF-8 text. |

If you keep a list in `kv` and the list can grow past some hundred items, use `sql`. A `kv` value is read and written as one string. There is no sync between devices yet (`docs/LIMITS.md`).

## 2. Declare the capabilities

Add one `[[capabilities]]` block to `plinth.toml` for each API that you use. Write a `rationale` in plain words. The consent screen shows it to the user.

```toml
[[capabilities]]
name = "store.kv"
rationale = "Save your settings on this device."

[[capabilities]]
name = "store.sql"
rationale = "Save your tasks on this device and search them."

[[capabilities]]
name = "files.private"
rationale = "Save your drafts on this device."
```

If you call `kv.get`, `sql.query` or `write` and do not declare the capability, `plinth check` gives the error `PL1007`. The app does not build.

All three capabilities have the risk level Low. The Hub allows them at install and does not ask the user. The user can refuse them later. Thus your app must work when a call is denied (§5).

## 3. Example

This example keeps settings in `kv` (as JSON text) and a draft in a file. It is checked with `plinth check` against `std/store.d.ts`, `std/files.d.ts` and `std/core.d.ts`.

```tsx
import { app, signal, Screen, Section, TextField, TextArea, Button, Text } from "plinth:ui";
import { kv } from "plinth:store";
import { JSON } from "plinth:core";
import { read, write } from "plinth:files";

interface Prefs {
  name: string;
  visits: number;
}

// kv: small values. Store an object as JSON text.
function loadPrefs(): Prefs {
  const raw = kv.get("prefs");
  if (raw === null) {
    return { name: "", visits: 0 };
  }
  const parsed = JSON.parse<Prefs>(raw);
  if (parsed === null) {
    return { name: "", visits: 0 };
  }
  return parsed;
}

function savePrefs(p: Prefs): void {
  kv.set("prefs", JSON.stringify(p));
  const why = kv.lastError();
  if (why !== null) {
    status.set("Not saved: " + why);
  }
}

const status = signal("");
const name = signal(loadPrefs().name);
const draft = signal("");

// files: larger text, in the private space of the app.
async function saveDraft(): Promise<void> {
  try {
    await write("drafts/today.md", draft());
    status.set("Draft saved.");
  } catch (e) {
    status.set("Draft not saved: " + (e as Error).message);
  }
}

async function openDraft(): Promise<void> {
  try {
    draft.set(await read("drafts/today.md"));
  } catch (e) {
    const why = (e as Error).message;
    status.set(why === "not-found" ? "No draft yet." : "Cannot open the draft: " + why);
  }
}

function Home() {
  return (
    <Screen title="Storage">
      <Section title="Settings">
        <TextField label="Your name" value={name} />
        <Button label="Save name" onPress={() => savePrefs({ name: name(), visits: loadPrefs().visits + 1 })} />
      </Section>
      <Section title="Draft">
        <TextArea label="Draft" value={draft} />
        <Button label="Save draft" role="primary" onPress={() => { saveDraft(); }} />
        <Button label="Open draft" onPress={() => { openDraft(); }} />
        <Text tone="muted">{status()}</Text>
      </Section>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Storage", component: Home } } });
```

### 3.1 SQL example

This example keeps tasks in a table. `sql.migrate` makes the table on the first run, and adds later changes of the schema in order.

```ts
import { sql } from "plinth:store";

interface Task {
  id: number;
  title: string;
  done: boolean;
}

// Append-only: add new steps at the end; never change a step that ran.
const SCHEMA = [
  "CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0)",
  "CREATE INDEX tasks_done ON tasks (done)",
];

export async function open(): Promise<void> {
  await sql.migrate(SCHEMA);
}

export async function add(title: string): Promise<number> {
  const r = await sql.exec("INSERT INTO tasks (title) VALUES (?)", [title]);
  return r.lastInsertId;
}

export async function openTasks(): Promise<Task[]> {
  return await sql.query<Task>("SELECT id, title, done FROM tasks WHERE done = ? ORDER BY id", [false]);
}

export async function finishAll(ids: number[]): Promise<void> {
  // One transaction: all rows change, or none.
  await sql.batch(ids.map((id) => ({ sql: "UPDATE tasks SET done = 1 WHERE id = ?", params: [id] })));
}
```

More examples: `examples/notes/app/storage.ts` (`store.sql` with a full-text search, and a one-time move of old kv data), `examples/files-demo` (files).

## 4. Rules for each API

### 4.1 `kv`

- `kv.get(key)` returns the value, or `null` if the key is not set.
- `kv.set(key, value)` replaces the value. `kv.remove(key)` removes it; a missing key is not an error.
- `kv.keys()` returns all keys, in no fixed order.
- To keep an object, use `JSON.stringify` and `JSON.parse<T>`. `JSON.parse<T>` returns `null` for invalid JSON or for a wrong shape. Always handle `null`: an older version of your app can have saved a different shape.

### 4.2 `sql`

- Write each statement as a string literal or a `const`, with `?` for each value. Do not build a statement from user text: the compiler gives the warning `PL2026`.
- A parameter is a `number`, a `string`, a `boolean` (stored as 0 or 1) or `null`. `BLOB` values are not supported.
- `query<T>` decodes each row into `T`, one field for each column. Give each column a name that is the same as the field name (`SELECT count(*) AS n`). A `boolean` field reads 0 and 1. A missing column or a wrong type rejects the promise.
- One call runs one statement. Use `batch` for a transaction. The host refuses `BEGIN`, `COMMIT`, `ATTACH`, `VACUUM`, extensions and most `PRAGMA`s (`docs/SQL.md` §4).
- Keep the steps of `sql.migrate` append-only. The database records how many steps ran (`PRAGMA user_version`). A list with fewer steps than the database has is an error.
- Full-text search (SQLite FTS5) is available on every host. To search user text, put each word in `"…"` and double each `"` in it. Then an FTS5 operator in the text is plain text. `examples/notes/app/storage.ts` shows this.

### 4.3 Files

- A path is relative, with `/` between segments, for example `notes/2026/today.md`. The host refuses absolute paths, `.` and `..`, empty segments, `\`, `:` and other characters. [host-apis.md](../host-apis.md) has the full list of path rules. The rules are the same on every host.
- `write` makes the parent folders. A folder exists only while it holds a file. There is no `mkdir`.
- `list("")` lists the root. `list` sorts the entries by name.
- `remove` removes a file, or a folder and all its contents.
- Write names in NFC. Names are case-sensitive on the web, and not on Windows and macOS.
- Each call also has a callback form: give `done(error, value)` as the last argument.

## 5. Handle a denied or failed call

A denied call never stops your app. Show the problem to the user as a normal state.

| API | On denial | How to find the reason |
|---|---|---|
| `kv` | `get` returns `null`, `set` and `remove` do nothing, `keys` returns `[]`. | `kv.lastError()` returns `"denied:undeclared"`, `"denied:refused"` or `"denied:unsupported"`, or `null` after a call that worked. Read it right after the call. |
| `sql` | The promise rejects with an `Error`. | `message` is `"denied:<reason>"`, or `"sql: <detail>"` for a SQL error, for example `"sql: the query took too long"`, `"sql: the database is full"` or a syntax error. |
| Files | The promise rejects with an `Error`. | `message` is the reason: `"denied:<reason>"`, `"invalid-path: <rule>"`, `"not-found"`, `"not-a-file"`, `"not-a-directory"`, `"not-text"`, `"too-large"`, `"quota"` or `"io: <detail>"`. |

An uncaught rejection does not stop the app. The host reports it as an uncaught error.

## 6. Limits

| Limit | Desktop | Web |
|---|---|---|
| `kv` size | No quota. | In a sandboxed frame (the web Hub, a web export): 512 Ki characters of keys and values for each app. A `set` over the quota does nothing and `kv.lastError()` returns `"denied:refused"`. A key longer than 1024 characters gets the same result. |
| One file | 8 MiB (`"too-large"`). | 8 MiB. |
| All files of one app | 50 MiB (`"quota"`). | 50 MiB. |
| One SQL database | 256 MiB (`"sql: the database is full"`). | 256 MiB. |
| One SQL statement | 2 s (`"sql: the query took too long"`). | 2 s. One SQLite Worker runs the calls of all apps of the page in turn. |
| One `query` result | 16 MiB of JSON. | 16 MiB. |

## 7. Where the data is

| Host | `kv` | `sql` | Files |
|---|---|---|---|
| Desktop | `%APPDATA%\plinth\apps\<app id>\kv.json` on Windows. One file for each app id. | `%APPDATA%\plinth\spaces\private\<owner>\<app id>+sql\store.sqlite`, next to the private space. The `files` API cannot reach it. | `%APPDATA%\plinth\spaces\private\<owner>\<app id>\`. `<owner>` comes from the publisher key of a signed package, from the package digest of an unsigned package (so each new build gets a new space), or is `dev` for `plinth dev`. |
| Web, in a sandboxed frame | The `localStorage` of the page, keys `plinth-hub:kv:<app id>:<key>`. | The OPFS of the page, folder `.plinth-sql`, one database for each app id. Without OPFS (an old browser), in memory. | The IndexedDB of the page, database `plinth-files`. |
| Web, stand-alone page (`web/index.html`, for tests) | `localStorage`, keys `plinth:<app id>:<key>`. | The same as in a sandboxed frame. | In memory. The files go away at a reload. |

Notes:

- On the desktop, the kv store is keyed by the app id only, and the private file space and the SQL database by the owner and the app id.
- On the web, only one browser tab of an origin at a time can open the SQL databases (`docs/SQL.md` §9 W5). A web export (`plinth build --target web`) does not have `sql` yet.
- On the desktop, the files of an unsigned package are in a new space after each build. Sign your package (see [publishing.md](publishing.md)) to keep the same space across versions.
- On the web, all data is in the storage of the page origin. The apps cannot read the data of other apps, but code of the page can (`docs/THREAT-MODEL.md` §6).
- [host-apis.md](../host-apis.md) "Support on each host" has the other differences between hosts.
