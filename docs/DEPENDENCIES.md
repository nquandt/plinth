# Dependencies: audit and licenses

This document tells how Plinth checks its dependencies for security advisories and licenses. It gives the license policy, each exception and its reason, and the list of licenses in use. `docs/PRODUCTION.md` items S8 and G4 point here.

## 1. How the audit runs

The audit has two parts:

- **`cargo deny check`** reads `deny.toml` at the repository root. It does four checks on the Rust dependency graph (`Cargo.lock`) for the targets that Plinth ships to (Windows, Linux, macOS, wasm32):
  - **advisories**: the RustSec advisory database. A vulnerability fails the check. An unmaintained crate and a yanked version give a warning.
  - **licenses**: each crate must have a license from the allow list (§2), or an exception (§3).
  - **bans**: a wildcard version (`*`) fails the check. Duplicate versions of a crate give a warning.
  - **sources**: crates come only from crates.io and from the git repositories in `deny.toml`. gpui-ce comes in by path (`../gpui-ce`).
- **`npm audit`** in `editors/vscode`. This is the only npm package with third-party packages (all of them are dev tools: `typescript`, `@types/node`, `@types/vscode`). The packages under `npm/` have no third-party dependencies: `npm/cli` has optional dependencies only on its own platform packages (`@plinth/cli-*`).

Commands:

```sh
bash scripts/ci-local.sh deny        # the audit only (cargo deny and npm audit)
bash scripts/ci-local.sh full        # the full gate; the audit is its first step
cargo deny check -W unmaintained -A unnecessary-skip   # cargo deny by hand, as the gate runs it
cargo deny check advisories          # one check
```

Install cargo-deny one time: `cargo install cargo-deny --locked`. If cargo-deny is not installed, `ci-local.sh` writes `SKIPPED` and continues, so the gate still runs on other machines. The same applies to npm.

CI: the Linux job installs a prebuilt cargo-deny (`taiki-e/install-action`) and runs `bash scripts/ci-local.sh deny` after the tests. The Windows job runs `full`, which includes the audit if the runner has cargo-deny.

Flags:

- `-W unmaintained`: cargo-deny 0.20 has no "warn" level for unmaintained crates in `deny.toml` (only a scope). The flag makes them warnings.
- `-A unnecessary-skip`: the `skip` entries for gpui-ce crates (§3, B1) are not for duplicates, so cargo-deny calls them unnecessary. The flag hides that note.

The advisory database changes every day. A new advisory can make the gate fail with no change in Plinth. Then do the steps in §4.

## 2. License policy

Plinth is MIT OR Apache-2.0. A dependency must have a permissive license, so that Plinth and the apps that its tools make can go out under MIT OR Apache-2.0.

Allowed for all crates (`deny.toml` `[licenses] allow`):

MIT, MIT-0, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, Unicode-DFS-2016, CC0-1.0, BSL-1.0, 0BSD, Unlicense.

Rules:

- Do not add a copyleft license (GPL, LGPL, AGPL, MPL, EPL, CDDL) to the allow list. Give one crate an exception with a reason (§3).
- A crate with no license field and no license file fails the check. If the license is clear from the repository, add a `[[licenses.clarify]]` entry with the source of the information.
- First-party crates (Plinth and gpui-ce) are in the graph. Plinth's crates are MIT OR Apache-2.0. gpui-ce's crates are Apache-2.0 (gpui-ce is a fork of Zed's gpui, which is Apache-2.0). Crates with `publish = false` are not checked.

A binary that contains Apache-2.0 code (for example the desktop host, which contains gpui-ce and wasmtime) must give the license text and any NOTICE files of those crates to the user. A generated third-party notice file for the release binaries is not done yet (§4, L1).

`plinth-cli` uses the `wasm-opt` crate (MIT OR Apache-2.0) for `plinth build --release`. Its `wasm-opt-sys` part contains the C++ source of Binaryen (Apache-2.0) and compiles it into the `plinth` binary with the `cc` and `cxx` crates. Thus a build of `plinth-cli` needs a C++17 compiler (MSVC on Windows, `g++` or `clang++` on Linux and macOS). The default feature `dwarf` (LLVM DWARF code) is off. The notice file (L1) must include Binaryen.

`plinth-runner-wasmtime` uses the `rusqlite` crate (MIT) for `store.sql` (`docs/SQL.md`), with the features `bundled`, `hooks` and `limits`. With `bundled`, its `libsqlite3-sys` part (MIT) compiles the C source of SQLite (public domain) into the host with the `cc` crate. Thus a build of the runner needs a C compiler (MSVC on Windows, `cc` on Linux and macOS). The bundled build has FTS5. The dependency is not behind a feature flag: a clean build of `libsqlite3-sys` takes about 11 s in a debug build and 17 s in a release build (Windows, 2026-10-08), and it compiles in parallel with other crates. For the wasm32 target, `rusqlite` also brings `sqlite-wasm-rs` and `rsqlite-vfs` (MIT) into the checked graph. The web host does not use them (it uses the official SQLite Wasm build, `docs/SQL.md` §2). The notice file (L1) must include SQLite.

## 3. Exceptions

Each exception is in `deny.toml` with a comment.

### Licenses

| Crate | License | Comes from | Reason |
|---|---|---|---|
| `dwrote` | MPL-2.0 | `zed-font-kit` (gpui-ce), Windows | DirectWrite font bindings. MPL-2.0 is file-level copyleft: Plinth can ship it in a binary if the MPL source files stay available (they are on crates.io). Plinth does not change these files. |
| `option-ext` | MPL-2.0 | `dirs-sys` ← `dirs` ← `zed-font-kit` (gpui-ce) | A small helper. Same reason as `dwrote`. |
| `webpki-roots` (0.26 and 1.0) | CDLA-Permissive-2.0 | `ureq` 2 (`plinth-registry`), `ureq` 3 (host, runner) | The Mozilla CA root certificates as data. CDLA-Permissive-2.0 is a permissive license for data. |

### License clarifications

| Crate | Expression | Reason |
|---|---|---|
| `wgsl-rs`, `wgsl-rs-ir`, `wgsl-rs-macros` | MIT | Git dependency of gpui-ce (`schell/wgsl-rs`, rev `b2de16a1`). The manifests have no license field. The repository has an MIT `LICENSE` file ("Copyright (c) 2025 Schell Carl Scivally"). |

### Bans

| Crate | Finding | Reason |
|---|---|---|
| `gpui_ce_apple`, `gpui_ce_render`, `gpui_ce_wgpu`, `gpui_ce_windows` | Wildcard: `wgsl-rs` is a git dependency with no version | gpui-ce pins `wgsl-rs` by commit, so the build is reproducible. These crates are in `[bans] skip`, which removes them from the wildcard and the duplicate checks only. See B1. |

Duplicate versions (about 65 crates, for example `windows-sys`, `hashbrown`, `wasmparser`, `syn`) are warnings only. Most come from gpui-ce and wasmtime, which use different versions of the same crates.

### Sources

| Source | Reason |
|---|---|
| `https://github.com/schell/wgsl-rs` | Git dependency of gpui-ce, pinned by rev. |
| `https://github.com/nquandt/gpui-ce` | For a checkout that uses gpui-ce from git in place of the sibling path. Not used now. |

### Advisories

| Advisory | Crate | Reason |
|---|---|---|
| RUSTSEC-2026-0316 (GHSA-jqpg-j7w6-42pr) | `wasmtime` 46.0.3 | Dynamic record lifting can allocate more than the hostcall fuel limit. The bug is in the dynamic `component::Val` API. Plinth calls components only through the typed bindings of `wasmtime::component::bindgen!` (no `component::Val` in the code), so the bug does not apply. See A1. |
| RUSTSEC-2026-0327 (GHSA-32h6-97mm-8q3c) | `wasmtime` 46.0.3 | The result count of async-lifted component callbacks is not validated. The bug needs the `component-model-async` feature. Plinth turns off the default features of wasmtime and does not turn on this feature (root `Cargo.toml`). See A1. |
| RUSTSEC-2024-0436 (warning only) | `paste` 1.0.15 | No longer maintained. It comes from `pulp` ← `exr` ← `image` ← gpui-ce. A proc macro at build time only. No action until `image` changes. |

## 4. Open items (tracking)

| # | Item | Next step |
|---|---|---|
| A1 | Update wasmtime from 46.0.3 to 48.0.4 or later. 46.x has no fix for RUSTSEC-2026-0316 and RUSTSEC-2026-0327. | A major update: the wasmtime API changes between majors, and `wit-bindgen`, `wit-component`, `wasmparser` and `wasm-encoder` must move with it. Do it as its own task with the full gate and the wasmi engine tests. Then remove the two `ignore` entries in `deny.toml`. |
| B1 | gpui-ce uses `wgsl-rs` as a git dependency with no version. | In gpui-ce: add a version to the `wgsl-rs` workspace dependency, or use a crates.io release when one exists. A git dependency also stops a publish of gpui-ce to crates.io. Then remove the `skip` entries in `deny.toml`. |
| L1 | Release binaries need a third-party license notice file. | Generate it at release time (for example with `cargo about`) and ship it next to the binary and in the npm packages. Add it to `docs/RELEASING.md`. |
| D1 | `plinth-registry` uses `ureq` 2; the other crates use `ureq` 3. | Move `plinth-registry` to `ureq` 3 (one less copy of `ureq`, `webpki-roots` and `rustls` parts). |

When the gate fails on a new advisory:

1. Read the advisory (`cargo deny check advisories`). Find the fixed versions.
2. If a fixed version is semver-compatible, run `cargo update -p <crate>` and run the full gate.
3. If the fix needs a major update, or the bug does not apply to Plinth, add an `ignore` entry with a reason to `deny.toml`, add a row to §3 and an open item to §4.

## 5. Licenses in use

This list is generated. To make it again after a change to `Cargo.lock`:

```sh
node scripts/gen-license-list.mjs --write
```

The script reads `cargo deny list` (the same graph and targets as `deny.toml`) and `cargo metadata`.

<!-- license-list:begin -->
Third-party crates in the checked graph: **766**.

Crates whose license expression names each license (a crate that names two licenses counts once for each):

| License | Crates |
|---|---|
| MIT | 669 |
| Apache-2.0 | 517 |
| Apache-2.0 WITH LLVM-exception | 52 |
| Zlib | 26 |
| Unicode-3.0 | 24 |
| BSD-3-Clause | 14 |
| Unlicense | 10 |
| BSD-2-Clause | 8 |
| ISC | 5 |
| BSL-1.0 | 4 |
| CC0-1.0 | 4 |
| CDLA-Permissive-2.0 | 2 |
| MPL-2.0 | 2 |
| 0BSD | 1 |
| MIT-0 | 1 |

Crates by license set (all licenses that the license expression of one crate names, joined by OR or AND):

| Licenses in the expression of one crate | Crates |
|---|---|
| Apache-2.0 / MIT | 447 |
| MIT | 158 |
| Apache-2.0 WITH LLVM-exception | 28 |
| Apache-2.0 / Apache-2.0 WITH LLVM-exception / MIT | 23 |
| Apache-2.0 / MIT / Zlib | 22 |
| Unicode-3.0 | 22 |
| BSD-3-Clause | 11 |
| Apache-2.0 | 10 |
| MIT / Unlicense | 10 |
| Apache-2.0 / BSD-2-Clause / MIT | 4 |
| BSD-2-Clause | 4 |
| Zlib | 4 |
| ISC | 3 |
| Apache-2.0 / BSD-3-Clause | 2 |
| Apache-2.0 / MIT / Unicode-3.0 | 2 |
| BSL-1.0 | 2 |
| CC0-1.0 | 2 |
| CDLA-Permissive-2.0 | 2 |
| MPL-2.0 | 2 |
| 0BSD / Apache-2.0 / MIT | 1 |
| Apache-2.0 / BSD-3-Clause / MIT | 1 |
| Apache-2.0 / BSL-1.0 | 1 |
| Apache-2.0 / CC0-1.0 | 1 |
| Apache-2.0 / CC0-1.0 / MIT-0 | 1 |
| Apache-2.0 / ISC | 1 |
| Apache-2.0 / ISC / MIT | 1 |
| Apache-2.0 WITH LLVM-exception / BSL-1.0 | 1 |

First-party crates (path dependencies; not counted above):

| License | Crates |
|---|---|
| Apache-2.0 (gpui-ce) | gpui-ce, gpui_ce_apple, gpui_ce_collections, gpui_ce_derive_refineable, gpui_ce_elements, gpui_ce_linux, gpui_ce_macos, gpui_ce_macros, gpui_ce_media, gpui_ce_parley, gpui_ce_platform, gpui_ce_refineable, gpui_ce_render, gpui_ce_scheduler, gpui_ce_shared_string, gpui_ce_sum_tree, gpui_ce_util, gpui_ce_web, gpui_ce_wgpu, gpui_ce_windows |
| MIT OR Apache-2.0 (Plinth) | counter-rs, plinth-cli, plinth-compiler, plinth-guest, plinth-host-desktop, plinth-hub, plinth-link, plinth-package, plinth-protocol, plinth-registry, plinth-rt, plinth-runner-wasmtime, plinth-shoot, plinth-ui, todo-rs |
<!-- license-list:end -->
