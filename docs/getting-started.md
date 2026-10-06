# Getting started

This page takes you from zero to a running app, step by step. It assumes
Windows, the only platform this project tests today.

## 1. Install

The npm packages (`@plinth/cli`, `create-plinth`) are not published yet
(see [RELEASING.md](RELEASING.md)). Build the `plinth` tool from source:

1. Install Rust (stable) and the Wasm target:
   ```sh
   rustup target add wasm32-unknown-unknown
   ```
2. Install Node 22 or later. The compiler does not need it to build an
   app, but the npm scripts and `tsc` checks in the examples use it.
3. Check out [`gpui-ce`](https://github.com/nquandt/gpui-ce) next to this
   repository, so it resolves as `../gpui-ce`:
   ```sh
   git clone https://github.com/nquandt/gpui-ce ../gpui-ce
   ```
4. Build the CLI:
   ```sh
   cargo build --release -p plinth-cli
   ```
   The binary is at `target/release/plinth.exe`. Put it on your `PATH`,
   or run it as `target/release/plinth` for the rest of this page.

Once the npm packages are published, this step becomes
`npm create plinth@latest my-app`, with no Rust toolchain needed.

## 2. Make your first app

```sh
plinth new my-app
cd my-app
```

`plinth new` writes:

```
my-app/
  plinth.toml         # app manifest
  tsconfig.json        # closed-world TypeScript config
  package.json          # npm scripts that call `plinth`
  app/
    main.tsx             # the app entry
```

`app/main.tsx` is a complete "hello world" with one screen and a counter.
Run it:

```sh
plinth dev
```

A window opens with your app. Edit `app/main.tsx`, save, and the window
reloads with your change. Signal state survives a reload when the
signal's shape does not change (see [language.md](language.md)).

## 3. Project layout

| Path | What |
|---|---|
| `plinth.toml` | The app id, name, version, publisher, and declared capabilities. It becomes `manifest.toml` inside the built package. |
| `tsconfig.json` | A closed-world TypeScript config: `"noLib": true`, `"types": []`, and `paths` that map `plinth:*` to the typings in `.plinth/types/`. Your editor and `tsc` use it, so you get real autocomplete and real errors. |
| `app/main.tsx` | The entry point: `export default app({ screens: {...} })`. |
| `app/*.tsx` | More screens and components. Import them with relative paths. |
| `assets/` | Images for `<Image src="...">` (UI API 1.3). |
| `.plinth/types/` | The `plinth:*` typings, written by `plinth` commands. Do not edit them by hand; they are regenerated. |

Every command below also accepts a project directory as its first
argument (`plinth check my-app`); it defaults to the current directory.

## 4. The commands you use every day

```sh
plinth dev                 # a window with hot reload
plinth check                # type-check and run the closed-world rules, no build
plinth check --json          # the same, as machine-readable output
plinth check --watch          # type-check again on every save
plinth build                 # writes dist/<name>.plnt
plinth run dist/<name>.plnt    # run the built package in the local host
plinth native dist/<name>.plnt -o MyApp.exe
                              # one executable: this host, a runtime core, and your app
```

`plinth check` catches two kinds of problem: ordinary TypeScript type
errors, and Plinth-specific rejections (unsupported syntax, undeclared
capabilities — see [language.md](language.md)). A clean project reports:

```
ok (0 errors, 0 warnings)
```

## 5. Add a second screen and navigate

Create `app/settings.tsx`:

```tsx
import { Screen, Section, Text } from "plinth:ui";

export default function Settings() {
  return (
    <Screen title="Settings">
      <Section>
        <Text tone="muted">Nothing to configure yet.</Text>
      </Section>
    </Screen>
  );
}
```

Register it in `app/main.tsx` and list it as a top-level destination:

```tsx
import { app, Screen } from "plinth:ui";
import Home from "./home";
import Settings from "./settings";

export default app({
  screens: {
    home: { title: "Home", icon: "house", component: Home },
    settings: { title: "Settings", icon: "gear", component: Settings },
  },
  primary: ["home", "settings"],
});
```

`primary` screens get a tab bar (narrow windows), a navigation rail
(medium), or a sidebar (wide) — the runtime picks the shell for the
window's width class. Inside a tab, push another screen onto the stack:

```tsx
import { navigate } from "plinth:ui";

navigate.push("settings");  // push "settings" onto the current tab's stack
navigate.back();             // pop it
navigate("home");             // switch the selected primary screen
```

The compiler checks every screen name you pass to `navigate` against
`app({ screens })`, so a typo is a compile error, not a runtime crash.

## 6. Save data with `plinth:store`

`plinth:store` is a simple per-app key-value store. It needs the
`store.kv` capability, so declare it in `plinth.toml` with a plain-English
reason the user will see on a consent screen:

```toml
[[capabilities]]
name = "store.kv"
rationale = "Save your notes on this device."
```

Then use it:

```tsx
import { app, signal, Screen, Section, Text, Button } from "plinth:ui";
import { kv } from "plinth:store";

function Home() {
  const note = signal(kv.get("note") ?? "nothing yet");
  return (
    <Screen title="Hello">
      <Section title="Store">
        <Text>{note()}</Text>
        <Button label="Save" role="primary" onPress={() => {
          kv.set("note", "saved!");
          note.set("saved!");
        }} />
      </Section>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", icon: "house", component: Home } } });
```

If you call `kv.get`/`kv.set`/`kv.remove`/`kv.keys` without declaring
`store.kv`, `plinth check` rejects it at compile time:

```
app/main.tsx(8,16): error PL1007: this call needs the `store.kv` capability, which `plinth.toml` does not declare
  help: add `[[capabilities]]` with `name = "store.kv"` and a `rationale` to plinth.toml (SPEC.md §11)
```

See [host-apis.md](host-apis.md) for the other host modules and how
capabilities and denial work at run time.

## 7. Package and check your app

```sh
plinth build
```

writes `dist/<name>.plnt`, a zip archive that holds only your app code
(a counter app is about 2 KB). Check it:

```sh
plinth validate dist/<name>.plnt
```

prints the declared and reachable capabilities, and fails if the package
is malformed or if it imports anything outside the Plinth runtime. Run
it:

```sh
plinth run dist/<name>.plnt
```

or make a single, double-clickable executable for people who do not have
a Plinth host installed:

```sh
plinth native dist/<name>.plnt -o MyApp.exe
```

This file bundles the desktop host, a runtime core, and your app. It is
much larger than the `.plnt` (tens of MB, mostly the host and the
renderer); the `.plnt` itself stays a few KB and is the file you would
publish to a hub or a registry.
