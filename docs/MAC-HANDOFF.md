# Handoff for the Mac: Plinth on an iPhone

> Status: 2026-10-27. Written on the Windows PC. Nothing in this file was tried on a Mac. This document uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

## 0. The goal

The owner builds small Plinth apps for personal use, publishes them to a personal registry, and installs them on an iPhone from a Plinth host. First install: Xcode developer mode (a cable and a free or paid Apple ID). Later: SideStore.

Read `HANDOFF.md` first, then `docs/REGISTRY-HOSTING.md` and `docs/PLATFORM-TODO.md`.

## 1. The facts that decide the plan

1. **The registry works now.** `plinth.femur.dev` (when the tunnel is up; `infra/registry/README.md`) serves a registry and the web App Hub, behind Cloudflare Access (§6). A Plinth app already runs in a phone browser through the web host. Safari is untested (PRODUCTION P4).
2. **There is no iOS window in gpui-ce.** `../gpui-ce/crates` has `gpui_macos` (AppKit), `gpui_apple` (the shared Metal renderer), `gpui_windows`, `gpui_linux`, `gpui_web` and `gpui_wgpu`. There is no `gpui_ios` and no UIKit code. The HANDOFF assumption "assume gpui mobile backends exist" is **wrong today**: somebody must write the iOS platform. This is the largest item on this list.
3. **The native host cannot start on iOS yet.** `plinth-runner-wasmtime` depends on `wasmtime`, which cannot run generated code on iOS (no JIT). The engine to use is `wasmi` (feature `wasmi`, passes all suites, 7 to 11 times slower than wasmtime in a release build). The host state builds without `wasmtime` since 2026-10-27 (PRODUCTION R5, §5.1); the Mac checks that it builds for iOS.
4. **Neither dev mode nor SideStore allows JIT.** A host that uses `wasmi` runs under both. Do not add the JIT entitlement.
5. **The macOS desktop window is untested too.** `plinth run examples/counter` has never run on macOS (PLATFORM-TODO P3). It is the first check, and the iOS work reuses most of its pieces.

## 2. Two tracks

| Track | What | Time | Follows owner decisions? |
|---|---|---|---|
| **A. Web shell** | `hosts/ios-shell`: a small Swift app that opens `https://plinth.femur.dev/` in a `WKWebView`. Real icon, full screen, installs with dev mode or SideStore. | Hours | The web host stays DOM (decision 3), and one `.plnt` runs everywhere. It is **not** the gpui host of decision 2, so it is a bridge. The owner decides. |
| **B. Native gpui host** | `gpui_ios` platform, engine-free host state, an Xcode app with a Rust static library, wasmi. | Weeks | Yes. |

Recommended order: do the macOS checks (§3), then track A to get apps on the phone, then track B step by step. Track A costs little and gives a way to use the apps while track B is built.

## 3. Day one on the Mac (checks before any new code)

1. Install: Xcode (and `xcode-select --install`), Homebrew, Rust (`rustup`), Node 22, `xcodegen`, `cloudflared` (only if the Mac serves the registry).
2. `rustup target add wasm32-unknown-unknown aarch64-apple-ios aarch64-apple-ios-sim`.
3. Get the code (§7): clone `plinth`, and clone `gpui-ce` as a sibling folder `../gpui-ce` at the commit in `.github/workflows/ci.yml` (`GPUI_CE_COMMIT`).
4. `bash scripts/ci-local.sh nogui`. The compiler, the runner, the Hub and the registry tests must pass. Write each failure in `docs/PLATFORM-TODO.md` before you fix it.
5. `cargo build -p plinth-cli` and `target/debug/plinth run examples/counter`. If the window opens, P3 starts to be Done. If it fails, fix it in `gpui-ce` or `plinth-host-desktop` (not in app code).
6. Open `https://plinth.femur.dev/` in macOS Safari and in the iPhone Safari. Run the examples. Write each difference in `docs/LIMITS.md` (P4). Check OPFS (`store.sql`), the Worker for each app, the sandboxed frame and the clipboard.

## 4. Track A: the web shell

Files: `hosts/ios-shell/` (`project.yml`, `Sources/PlinthHubApp.swift`). It was written on Windows and never built.

1. `cd hosts/ios-shell && xcodegen generate && open PlinthHub.xcodeproj`.
2. Select the team (Signing and Capabilities). A free Apple ID works; the profile lasts seven days.
3. On the iPhone: Settings, Privacy and Security, Developer Mode, On (it restarts the phone). Connect with a cable, trust the Mac, press Run in Xcode. Trust the developer profile on the phone (General, VPN and Device Management).
4. Check: the Hub opens, an app installs from the registry, the app keeps its data after you close and open the shell, `store.sql` works (OPFS in `WKWebView`), the keyboard does not hide the text field.
5. Fix what fails. A `WKWebView` is Safari's engine, so any problem is a web-host problem: fix it in `web/` (then `node scripts/gen-plinth-js.mjs`).
6. For SideStore, build an unsigned `.ipa`: `xcodebuild -scheme PlinthHub -sdk iphoneos -configuration Release -archivePath build/PlinthHub.xcarchive archive CODE_SIGNING_ALLOWED=NO`, then zip `Payload/PlinthHub.app` into an `.ipa`. SideStore signs it with the owner's Apple ID and refreshes it every seven days. A free account allows three apps.

## 5. Track B: the native host, in order

Each step builds and tests alone. Give one step at a time to one agent (`HANDOFF.md` §8).

1. **R5, host state without wasmtime.** Move the shared host logic (`HostState`, `Policy`, kv, timers, net, files, sql, locale, notify) so that it builds with `--no-default-features` and only the `wasmi` engine. **Done on Windows (2026-10-27):** `wasmtime` is now an optional dependency of `plinth-runner-wasmtime` (feature `wasmtime`, on by default). A split into a new crate `plinth-host-state` was not needed. Test: `cargo test -p plinth-runner-wasmtime --no-default-features --features wasmi`, and `cargo build --target aarch64-apple-ios -p <crate>`. Check that `rusqlite` (bundled SQLite), `ring`/`rustls` and `icu` build for iOS.

   **Design (R5).** One crate, not two. `HostState` and all host APIs stay in `lib.rs`. The wasmtime code moved to `src/wasmtime_engine.rs` (feature `wasmtime`): the engine, the epoch ticker, the call and the trap reasons. The wasmi code stays in `src/wasmi_engine.rs` (feature `wasmi`) and has no wasmtime type. The host traits of the `plinth:app` world (`bindings::plinth::app::*::Host`) come from `wasmtime::component::bindgen!` with the `wasmtime` feature. Without it, `src/host_traits.rs` has the same traits and types, written by hand. When `wit/plinth/app.wit` changes, change `host_traits.rs` too; the build `--no-default-features --features wasmi` fails if they differ. The crate needs at least one engine (`compile_error!`). Without the `wasmtime` feature, `Runner::new` always makes the wasmi engine. The other crates did not change: their `plinth_runner_wasmtime::...` paths are the same, and the default features are the same. The time zone offset (`timezone-offset`) uses the `time` crate everywhere except iOS, where `time` does not build (below); iOS calls `localtime_r`.

   **What was checked on Windows (2026-10-27).**

   | Command | Result |
   |---|---|
   | `cargo build -p plinth-runner-wasmtime --no-default-features --features wasmi` | Builds, no warnings. |
   | `cargo tree -p plinth-runner-wasmtime --no-default-features --features wasmi` | No `wasmtime`, `cranelift` or `pulley` crate (the only match for "wasmtime" is the name of the crate itself). |
   | `cargo test -p plinth-runner-wasmtime --no-default-features --features wasmi` | 77 passed. |
   | `cargo test -p plinth-runner-wasmtime` and `... --features wasmi` | 75 and 77 passed. |
   | `cargo test -p plinth-compiler --test secrets --test notify`, with and without `PLINTH_ENGINE=wasmi` | 7 and 14 passed, both ways. |
   | `cargo check --keep-going --target aarch64-apple-ios -p plinth-runner-wasmtime --no-default-features --features wasmi` | Stops at `ring 0.17.14` and `libsqlite3-sys 0.38.2` (bundled SQLite): their build scripts need the Apple SDK (`xcrun`, `clang`), which this PC has not. This is expected. One Rust failure was found and fixed: `num_threads 0.1.7` (from `time` with `local-offset`) does not compile for iOS (`libc::mach_task_self` is macOS only). It is out of the iOS dependency tree now. No other crate before the runner failed. |
   | The same with `--target aarch64-linux-android` | The same two crates stop, here because the Android NDK clang is missing. |
   | The same with `--target wasm32-unknown-unknown` | `ring` stops (the host state is not a web target; only the target was tried). |
   | `rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-linux-android` | Installed. |

   **Not checked.** The Rust code of `plinth-runner-wasmtime` itself was not type-checked for iOS, because its C dependencies stop the build first. This includes the `localtime_r` function for iOS (`local_offset_minutes` in `lib.rs`). The Mac must run `cargo check --target aarch64-apple-ios -p plinth-runner-wasmtime --no-default-features --features wasmi` and `cargo build` for `aarch64-apple-ios-sim` (`docs/PLATFORM-TODO.md`). Also not checked: that `rusqlite` (bundled SQLite) and `ring` build for iOS, and a run of the wasmi tests on a simulator.
2. **Hub without desktop code.** `plinth-hub` and `plinth-registry` must build for iOS (the registry server part can be a feature that iOS turns off). Check `ureq` + `rustls` against the iOS trust store.

   **Findings on Windows (2026-10-27; `cargo tree --target aarch64-apple-ios`, nothing changed).**

   | Crate | wasmtime in its tree? | Desktop-only code | Note |
   |---|---|---|---|
   | `plinth-protocol`, `plinth-package`, `plinth-link` | No | None | Pure Rust (`sha2`, `serde`, `ed25519-dalek`, `zip`). |
   | `plinth-runner-wasmtime` (wasmi only) | No | None | C code: `rusqlite` (bundled SQLite) and `ring` (through `rustls`). Needs the Apple SDK on the Mac. `ureq` 3 has the roots of `webpki-roots` built in, so it does not read the iOS trust store. |
   | `plinth-hub` | **Yes** | `windows`, `windows-registry` (only `cfg(windows)`, so absent on iOS) | It depends on `plinth-runner-wasmtime` with the default features. Fix: set `default-features = false` on the workspace entry of the runner and let `plinth-host-desktop`, `plinth-cli`, `plinth-shoot` and the hub/registry dev dependencies (tests) ask for `wasmtime`. |
   | `plinth-registry` | **Yes** (through `plinth-hub`; also a dev dependency) | None | Uses `ureq` 2 with `tls` (`rustls`, `ring`, `webpki-roots`) next to the `ureq` 3 of the runner: two versions of `ureq`, `ring` and `rustls` stay in the build. |
   | `plinth-host-desktop` | Yes | `gpui`, `windows`, `keyring` | Not an iOS crate: `plinth-host-ios` replaces it. Its `plinth:secrets` backend uses `keyring` 4.2 with feature `v1`, which on iOS selects `apple-native-keyring-store/keychain` (the Keychain). The SecretsBackend trait stays in the runner, so the host state does not depend on it. |
   | `icu_*` 2.3 | No | None | Pure Rust with compiled data (large, but no C code). |

   The hub and registry crates need no code change for iOS, only the `default-features = false` change above. The other agent that edits them should make it, or the owner decides who does.
3. **`gpui_ios` platform crate** in `../gpui-ce` (a new crate next to `gpui_macos`; wire it in `gpui_platform` with `cfg(target_os = "ios")`). It needs: a UIKit window and a `CAMetalLayer` view; a `CADisplayLink` frame loop; touch input mapped to pointer events (tap, drag, long press for `onContextMenu`, scroll with momentum); the software keyboard and text input (`UITextInput`, with IME composition); safe-area insets; the scale factor; dark mode, text size and reduced motion from `UITraitCollection` (PRODUCTION U7); the clipboard (`UIPasteboard`); the text system (CoreText; reuse `gpui_macos/src/text_system.rs`); the renderer (reuse `gpui_apple/src/metal_renderer.rs`, or `gpui_wgpu` with Metal); accessibility (AccessKit has an iOS adapter, for VoiceOver). Start from `gpui_macos` and delete what iOS lacks. Run a plain gpui example on the simulator first.
4. **`plinth-host-ios`**: a `staticlib` crate with a small C API (`plinth_host_main(...)`) that starts gpui on the UIKit app, runs the Hub app (`examples/hub`) with `wasmi`, and keeps data in the app container (`Application Support`, not shared with other apps). The Xcode project is a thin Swift `@main` that calls the C function.
5. **Install and updates.** Ship cores in the registry (`--with-core`) and let the host install them (`docs/UPDATES.md`). A native update is needed only when the host interface changes. Update by SideStore refresh.
6. **Notifications (H3), files picker, share sheet** as host APIs, each with the test table of `docs/host-apis.md`.
7. **Tests.** A simulator run of `examples/counter` and `examples/hub`; frame cost on a real phone (`PLINTH_TRACE_RENDER=1`); a wasmi speed check (the 60 fps budget in `docs/GAPS.md` "wasmi engine").

When gpui works on iOS, retire the shell of track A, or keep it as a fallback.

## 6. The services (already made)

| Thing | Where | Notes |
|---|---|---|
| Registry source folder | The PC: `PLINTH_REGISTRY_DIR` (default `~/plinth-registry`) | Not in git. Copy it or publish again from the Mac. |
| Publish script | `scripts/registry-publish.sh <app dir>... [--sync]` | Needs the `plinth` CLI, `az` for `--sync`. |
| Docker site and tunnel | `infra/registry/` | Needs `.env` with `TUNNEL_TOKEN` (a secret; not in git). |
| Azure | Resource group `plinth-rg`, storage account `plinthreg36610`, `centralus` | `az login`, then `bash scripts/registry-publish.sh ... --sync` works from the Mac. The account name is in `infra/azure/.account-name` (not in git): run `echo plinthreg36610 > infra/azure/.account-name`. |
| Access | `plinth.femur.dev` is behind Cloudflare Access (`docs/REGISTRY-HOSTING.md` §6) | Safari and the iOS shell: log in once with the owner e-mail. Native hosts need a service token: `scripts/secrets-pull.sh mac` restores `~/.plinth-access/mac.env` (use `iphone` for the phone). The Hub client does not send the token yet. |
| Keys | The PC: `%APPDATA%\plinth\publisher` (publisher "Nate"), `~/.plinth-hub-key` (Hub key, name "you") | **Not in git. They are in Azure Key Vault `plinthkv36610`: run `az login`, then `bash scripts/secrets-pull.sh mac`** (`docs/REGISTRY-HOSTING.md` §6.2). Do not send them by chat or e-mail to `~/.config/plinth/publisher` and `~/.plinth-hub-key` on the Mac, or make new keys: the registry then refuses a new version of an app that a different key signed. |

## 7. How the Mac gets the code

- `plinth`: `https://github.com/nquandt/plinth` (private). `master` was pushed on 2026-10-27.
- `gpui-ce`: `https://github.com/nquandt/gpui-ce`, commit `30572650c80c1975c33bfd66c90ae66ea00ee1cc`, on the branch `plinth-pin` (pushed on 2026-10-27; the remote `feat/webview` has other commits, so it was not used). Run `git clone https://github.com/nquandt/gpui-ce ../gpui-ce && git -C ../gpui-ce checkout 30572650c8`.
- If a push is not possible, make a bundle on the PC (`git bundle create plinth.bundle --all`) and copy it.

## 8. Order of the work, short

1. Mac checks (§3).
2. Track A on the phone (§4). Publish your own first app.
3. R5 (§5.1), then the Hub and registry for iOS (§5.2).
4. `gpui_ios` (§5.3). Plan this one with the owner before an agent starts: it is large and it is in the fork `gpui-ce`.
5. `plinth-host-ios` and the SideStore build (§5.4, §4.6).
