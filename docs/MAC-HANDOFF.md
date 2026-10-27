# Handoff for the Mac: Plinth on an iPhone

> Status: 2026-10-27. Written on the Windows PC. Nothing in this file was tried on a Mac. This document uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

## 0. The goal

The owner builds small Plinth apps for personal use, publishes them to a personal registry, and installs them on an iPhone from a Plinth host. First install: Xcode developer mode (a cable and a free or paid Apple ID). Later: SideStore.

Read `HANDOFF.md` first, then `docs/REGISTRY-HOSTING.md` and `docs/PLATFORM-TODO.md`.

## 1. The facts that decide the plan

1. **The registry works now.** `plinth.femur.dev` (when the tunnel is up; `infra/registry/README.md`) serves a registry and the web App Hub. A Plinth app already runs in a phone browser through the web host. Safari is untested (PRODUCTION P4).
2. **There is no iOS window in gpui-ce.** `../gpui-ce/crates` has `gpui_macos` (AppKit), `gpui_apple` (the shared Metal renderer), `gpui_windows`, `gpui_linux`, `gpui_web` and `gpui_wgpu`. There is no `gpui_ios` and no UIKit code. The HANDOFF assumption "assume gpui mobile backends exist" is **wrong today**: somebody must write the iOS platform. This is the largest item on this list.
3. **The native host cannot start on iOS yet.** `plinth-runner-wasmtime` depends on `wasmtime`, which cannot run generated code on iOS (no JIT). The engine to use is `wasmi` (feature `wasmi`, passes all suites, 7 to 11 times slower than wasmtime in a release build). The host state must build without `wasmtime` (PRODUCTION R5, open).
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

1. **R5, host state without wasmtime.** Move the shared host logic (`HostState`, `Policy`, kv, timers, net, files, sql, locale, notify) so that it builds with `--no-default-features` and only the `wasmi` engine. Today the crate `plinth-runner-wasmtime` has `wasmtime` as a hard dependency; make it optional (feature `wasmtime`, on by default), or split a crate `plinth-host-state`. Test: `cargo test -p plinth-runner-wasmtime --no-default-features --features wasmi`, and `cargo build --target aarch64-apple-ios -p <crate>`. Check that `rusqlite` (bundled SQLite), `ring`/`rustls` and `icu` build for iOS.
2. **Hub without desktop code.** `plinth-hub` and `plinth-registry` must build for iOS (the registry server part can be a feature that iOS turns off). Check `ureq` + `rustls` against the iOS trust store.
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
| Keys | The PC: `%APPDATA%\plinth\publisher` (publisher "Nate"), `~/.plinth-hub-key` (Hub key, name "you") | **Not in git. Copy them by a safe way** (not by chat or e-mail) to `~/.config/plinth/publisher` and `~/.plinth-hub-key` on the Mac, or make new keys: the registry then refuses a new version of an app that a different key signed. |

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
