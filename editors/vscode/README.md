# Plinth for VS Code

A minimal extension: on opening or saving a `.ts`/`.tsx` file inside a
folder with `plinth.toml` (searched upward from the file), it runs
`plinth check --json <project dir>` and publishes the results as
diagnostics, plus a status-bar item ("Plinth: ok" or "Plinth: N errors").

It finds the `plinth` binary in this order:

1. The `plinth.path` setting, if set.
2. `node_modules/.bin/plinth` in the project directory.
3. `plinth` on `PATH`.

## Build

No bundler: plain `tsc`.

```sh
npm install
npm run build     # tsc -p . -> out/
npm test          # node --test, the JSON -> diagnostic mapping only (no VS Code)
```

To try it in a dev host, open this directory in VS Code and press F5
(Run Extension).

## Package

```sh
npx -y @vscode/vsce package
```

This makes `plinth-vscode-<version>.vsix`. Install it with
`code --install-extension plinth-vscode-<version>.vsix`, or through the
"Install from VSIX..." command in the Extensions view. This extension is
not published to the Marketplace.

## Diagnostic mapping

`src/mapDiagnostics.ts` has no dependency on the `vscode` module, so
`test/mapDiagnostics.test.js` (plain `node --test`) covers it without a VS
Code host. It parses `plinth check --json`'s array of
`{ file, line, column, start, end, severity, code, message, help }`
objects (`crates/plinth-compiler/src/diag.rs`, `to_json`) into 0-based
ranges. `line`/`column` are 1-based and mark only the start of a span, so a
diagnostic that spans more than one line is clamped to the rest of its
start line; that is a known limitation, not a bug.
