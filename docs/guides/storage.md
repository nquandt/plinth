# Guide: save data

This guide tells you how to save data in a Plinth app. It is for app authors. The reference is [host-apis.md](../host-apis.md) (`plinth:store`, `plinth:files`).

## 1. Choose the API

Plinth gives an app two places for its data. Both are private to the app: no other app can read them.

| API | Use it for | Capability | Calls |
|---|---|---|---|
| `plinth:store` (`kv`) | Small values: settings, a list of short items, the last screen. | `store.kv` | Synchronous. A value is a string. |
| `plinth:files` | Larger text: documents, notes, exports. | `files.private` | Asynchronous. Each call returns a `Promise`. A file is UTF-8 text. |

There is no SQL store. There is no sync between devices yet (`docs/LIMITS.md`).

## 2. Declare the capabilities

Add one `[[capabilities]]` block to `plinth.toml` for each API that you use. Write a `rationale` in plain words. The consent screen shows it to the user.

```toml
[[capabilities]]
name = "store.kv"
rationale = "Save your settings on this device."

[[capabilities]]
name = "files.private"
rationale = "Save your drafts on this device."
```

If you call `kv.get` or `write` and do not declare the capability, `plinth check` gives the error `PL1007`. The app does not build.

Both capabilities have the risk level Low. The Hub allows them at install and does not ask the user. The user can refuse them later. Thus your app must work when a call is denied (§5).

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

More examples: `examples/notes/app/storage.ts` (kv with JSON), `examples/files-demo` (files).

## 4. Rules for each API

### 4.1 `kv`

- `kv.get(key)` returns the value, or `null` if the key is not set.
- `kv.set(key, value)` replaces the value. `kv.remove(key)` removes it; a missing key is not an error.
- `kv.keys()` returns all keys, in no fixed order.
- To keep an object, use `JSON.stringify` and `JSON.parse<T>`. `JSON.parse<T>` returns `null` for invalid JSON or for a wrong shape. Always handle `null`: an older version of your app can have saved a different shape.

### 4.2 Files

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
| Files | The promise rejects with an `Error`. | `message` is the reason: `"denied:<reason>"`, `"invalid-path: <rule>"`, `"not-found"`, `"not-a-file"`, `"not-a-directory"`, `"not-text"`, `"too-large"`, `"quota"` or `"io: <detail>"`. |

An uncaught rejection does not stop the app. The host reports it as an uncaught error.

## 6. Limits

| Limit | Desktop | Web |
|---|---|---|
| `kv` size | No quota. | In a sandboxed frame (the web Hub, a web export): 512 Ki characters of keys and values for each app. A `set` over the quota does nothing and `kv.lastError()` returns `"denied:refused"`. A key longer than 1024 characters gets the same result. |
| One file | 8 MiB (`"too-large"`). | 8 MiB. |
| All files of one app | 50 MiB (`"quota"`). | 50 MiB. |

## 7. Where the data is

| Host | `kv` | Files |
|---|---|---|
| Desktop | `%APPDATA%\plinth\apps\<app id>\kv.json` on Windows. One file for each app id. | `%APPDATA%\plinth\spaces\private\<owner>\<app id>\`. `<owner>` comes from the publisher key of a signed package, from the package digest of an unsigned package (so each new build gets a new space), or is `dev` for `plinth dev`. |
| Web, in a sandboxed frame | The `localStorage` of the page, keys `plinth-hub:kv:<app id>:<key>`. | The IndexedDB of the page, database `plinth-files`. |
| Web, stand-alone page (`web/index.html`, for tests) | `localStorage`, keys `plinth:<app id>:<key>`. | In memory. The files go away at a reload. |

Notes:

- On the desktop, the kv store is keyed by the app id only, and the private file space by the owner and the app id.
- On the desktop, the files of an unsigned package are in a new space after each build. Sign your package (see [publishing.md](publishing.md)) to keep the same space across versions.
- On the web, all data is in the storage of the page origin. The apps cannot read the data of other apps, but code of the page can (`docs/THREAT-MODEL.md` §6).
- [host-apis.md](../host-apis.md) "Support on each host" has the other differences between hosts.
