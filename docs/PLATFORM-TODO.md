# Work for Linux and macOS machines

This file lists the work that cannot be done on the Windows development PC. The owner will run agents on Linux and macOS machines later. Each item names its checklist id in `docs/PRODUCTION.md`. When an item is done, mark it here and in `PRODUCTION.md`.

Status: 2026-10-08. Nothing in this file is started.

## 1. Before an agent starts

1. Clone the repository. Read `HANDOFF.md` and `CONTRIBUTING.md`.
2. Install the build tools:
   - Linux: `infra/self-hosted-runner/setup-linux.sh` (it installs `build-essential` and the other packages).
   - macOS: Xcode command line tools (`xcode-select --install`), Rust, Node 22.
3. Run `bash scripts/ci-local.sh full`. The compiler, the runner and the Hub tests must pass already (`docs/LIMITS.md`). Record each failure as a new item in this file before you fix it.
4. Give an agent one bounded item at a time, and run only one build agent on a machine (`HANDOFF.md` §8, `docs/HISTORY.md`).

## 2. Linux

| Item | Id | What to do | Done when |
|---|---|---|---|
| A GUI window | P2 | Make `plinth-host-desktop` open a gpui-ce window on Linux (X11 and Wayland). Fix the build and the run, not the app code. | `plinth run examples/counter` opens a window. |
| The desktop tests | P2 | Run `plinth-shoot` (screenshots, `a11y_controls`, `context_menu`, `frame_cost`) on Linux. Golden images can differ by font: decide if Linux gets its own golden images or a tolerance. | The `plinth-shoot` tests pass in the gate on Linux. |
| Accessibility | U3 | Check the AccessKit tree with Orca (AT-SPI). | A manual check with Orca is recorded in `docs/guides/accessibility.md`. |
| Notifications | H3 | A `NotifyBackend` for the freedesktop notification service (D-Bus), in `crates/plinth-host-desktop/src/notify.rs`. Today `show` answers "unsupported". | `examples/notify-demo` shows a notification. |
| System settings | U7 | Dark mode, high contrast, text scale and reduced motion from the desktop settings (the XDG settings portal). | The theme follows each setting. |
| Secrets | H7 | A `SecretsBackend` for the Secret Service (libsecret). With no service, the call answers `unsupported`. | `examples/secrets-demo` keeps and returns a secret. |
| UI API 1.14 | U3 | Check `hidden`, `pressed`/`selected` (AT-SPI), `Screen layout="full"` and drag auto-scroll in `examples/kanban`. | As for macOS. |
| Locale change | L5 | The OS locale is read at start and when the window comes to the front. Check `sys-locale` on Linux. | `tests/locale.rs` passes; a manual locale change works. |
| SQL and files | H4 | Check the data folders (`$XDG_DATA_HOME/plinth/...`) and the `store.sql` database path. | The `sql`, `notes` and `files` suites pass. |
| One package on all OSes | P6 | Run the same `.plnt` files that Windows runs, with the same results. | The `apps`, `e2e` and `sevenguis` suites pass, and a native export starts with a double-click. |
| The release pipeline | D2 | Build and test the Linux release archive (`docs/RELEASING.md`). | A release build from CI runs on a clean Linux machine. |
| The CI runner | Q2 | Bring the self-hosted Linux runner online (`infra/self-hosted-runner/README.md`). | CI runs on each push to Linux. |
| Fuzz tests on a schedule | S9 | A scheduled CI job with more iterations of `robustness.rs`, `hostile_ops.rs` and `hostile_render.rs`. A Linux runner is the best place for it. | The job runs each night. |

## 3. macOS

| Item | Id | What to do | Done when |
|---|---|---|---|
| A GUI window | P3 | Make `plinth-host-desktop` open a gpui-ce window on macOS (Metal). | `plinth run examples/counter` opens a window. |
| The desktop tests | P3 | Run `plinth-shoot` on macOS, as for Linux. | The `plinth-shoot` tests pass in the gate on macOS. |
| Accessibility | U3 | Check the AccessKit tree with VoiceOver. | A manual check with VoiceOver is recorded in `docs/guides/accessibility.md`. |
| Text input with IME | U5 | Check Japanese and Chinese input in the gpui-ce text field. (Windows is checked on the Windows PC.) | Composition works in `TextField` and `TextArea`. |
| Notifications | H3 | A `NotifyBackend` for the macOS notification center (`UNUserNotificationCenter`; it needs a signed app bundle). | `examples/notify-demo` shows a notification. |
| System settings | U7 | Dark mode, increased contrast, text size and reduced motion from the macOS settings. | The theme follows each setting. |
| Secrets | H7 | A `SecretsBackend` for the macOS Keychain (`docs/SECRETS.md` §4); the same for iOS. | `examples/secrets-demo` keeps and returns a secret; `plinth hub uninstall` removes it. |
| UI API 1.14 | U3 | Check `hidden`, `pressed`/`selected` with VoiceOver, `Screen layout="full"` and drag auto-scroll in `examples/kanban`. | The Kanban board shows 4 columns at 1280 px, and a drag at the edge scrolls. |
| Signing and notarization | P5 | Sign and notarize the `plinth` binary and native exports (Apple Developer ID; the owner's account). | Gatekeeper starts the binary with no warning. |
| One package on all OSes | P6 | As for Linux. | As for Linux. |
| The release pipeline | D2 | Build, sign and test the macOS release (both `aarch64` and `x86_64`, or a universal binary). | A release build from CI runs on a clean Mac. |
| CI for each release | Q3 | Run CI on macOS for each release, not only weekly. | The release workflow needs the macOS job. |
| Safari | P4 | Run the web tests (`web/test/run-a11y.mjs` and the Node tests with a browser) in Safari. Check OPFS (`store.sql`), the Worker for each app, the sandboxed frame and `contextmenu` (Safari on iOS does not send it). | The web tests pass in Safari, or each difference is in `docs/LIMITS.md`. |
| iOS | P8, R5 | Later, after the macOS window: check the engine-free host state (R5; the Windows part is done): run `cargo check --target aarch64-apple-ios -p plinth-runner-wasmtime --no-default-features --features wasmi` and the same for `aarch64-apple-ios-sim`; fix what fails in Rust code (the `localtime_r` function `local_offset_minutes` in `lib.rs` was never compiled for iOS); check that `rusqlite` (bundled SQLite) and `ring` build; then `default-features = false` for the runner in `plinth-hub` and `plinth-registry` (`docs/MAC-HANDOFF.md` §5.1, §5.2). Then gpui on iOS with wasmi. | `docs/PRODUCTION.md` P8, R5. |

## 4. Any machine

| Item | Id | What to do |
|---|---|---|
| Chrome and Firefox | P4 | Run `web/test/run-a11y.mjs` in Chrome and Firefox, not only Edge. The test uses the DevTools protocol; Firefox needs WebDriver BiDi or Playwright. |
| Android | P7 | Later: gpui on Android with wasmtime. |
| A release key | S6 | The owner's offline root key and a CI subkey (owner decision). The owner does the key ceremony. |
| npm packages | D1 | Publish `@plinth/cli`, the platform packages and `create-plinth` when the platform builds exist. |
