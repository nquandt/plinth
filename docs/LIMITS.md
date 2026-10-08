# Known limits

This page tells what Plinth cannot do yet. Read it before you start a production app. `docs/PRODUCTION.md` has the plan for each limit (the item id is in the last column). `docs/GAPS.md` has the technical details.

Plinth is not ready for production apps. The current level is "before Preview".

## Platforms

| Limit | Item |
|---|---|
| Windows is the only platform with a GUI window. On Linux and macOS, the compiler, the runner and the Hub work, but no window opens. | P2, P3 |
| iOS and Android are not available yet. | P7, P8 |
| The web host is tested only in Microsoft Edge. Chrome, Firefox and Safari are not tested. | P4 |
| The Windows and macOS binaries are not signed. The OS can show a warning when you start them. | P5 |
| The npm packages (`@plinth/cli`, `create-plinth`) are not published. Build the CLI from source (`CONTRIBUTING.md`). | D1 |

## Language

| Limit | Item |
|---|---|
| Regular expressions have no lookaround and no backreferences. | L4 |
| No locale-aware formatting. Dates and numbers use English formats only. | L5 |
| An expression or a nesting deeper than 1000 levels gives `PL1000`. | L6 |
| No shared libraries between apps. Each app has its own copy of shared code. | L8 |
| A trap gives its source file and line only in a debug build (`plinth build` with no `--release`), on wasmtime and on the web. A release build, the wasmi engine and the web time limit give the reason with no place. The place is a line, with no column. An uncaught `Promise` rejection has no place. | L9 |

## User interface

| Limit | Item |
|---|---|
| No translations of app strings and no right-to-left layout. | U8, U9 |
| No context menu (secondary button), no drag and drop between controls, and no modal overlay that blocks the pointer on the desktop. | U6 |
| IME composition (for example Chinese or Japanese input) is not tested on the desktop host. The web host uses the browser's input fields. | U5 |
| The app does not follow all system settings yet: high contrast, text scale and reduced motion. | U7 |
| The desktop accessibility tree is not checked with a screen reader yet (Narrator, VoiceOver, Orca). On the desktop, a text field has its name on the group around it, not on the text input itself. | U3 |
| No animated transitions and no sound. | U10, U11 |
| Each nested flex box about doubles the layout work of its subtree on the desktop. Keep layouts flat. | R7 |

## Host APIs and storage

| Limit | Item |
|---|---|
| Notifications show on Windows and on the web. On Linux and macOS, `plinth:notify`'s `show` answers "unsupported". On the web, the browser asks for the permission; some browsers ask only after a click, and then `show` answers "unsupported". | H3 |
| Storage is a key-value store and a private file space. There is no SQL store. | H4 |
| No sync of app data between the devices of one user. | H6 |
| No camera, microphone or location. | SPEC §11 |
| Hot reload keeps signals with simple values only. Signals with arrays, objects, `Map` or `Set` start again. | SPEC §13 |

## Runtime and safety

| Limit | Item |
|---|---|
| The `wasmi` engine (for iOS) is 7 to 11 times slower than wasmtime (release build). Its time limit counts work (fuel), not time: a call stops after 1.2 s to 4.3 s of work, not after exactly 2 s. | R4 |

## Updates

| Limit | Item |
|---|---|
| A self-update cannot run app code to move data to a new format (`onUpgrade(fromVersion)` does not exist). | D4 |
| The restart prompt for `apply = "prompt"` is on the desktop host only. | D4 |
| Web apps have no offline update cache. | D5 |

## Developer tools

| Limit | Item |
|---|---|
| No test kit for app authors, no debugger and no inspector. | T3, T4 |
| The VS Code extension is not in the Marketplace. Install it from `editors/vscode`. | T2 |
