# Dependencies: audit and licenses

This document tells how Plinth checks its dependencies for security advisories and licenses. It gives the license policy, each exception and its reason, and the list of licenses in use. `docs/PRODUCTION.md` items S8 and G4 point here.

## 1. How the audit runs

The audit has two parts:

- **`cargo deny check`** reads `deny.toml` at the repository root. It does four checks on the Rust dependency graph (`Cargo.lock`) for the targets that Plinth ships to (Windows, Linux, macOS, wasm32):
  - **advisories**: the RustSec advisory database. A vulnerability fails the check. An unmaintained crate and a yanked version give a warning.
  - **licenses**: each crate must have a license from the allow list (§2), or an exception (§3).
  - **bans**: a wildcard version (`*`) fails the check. Duplicate versions of a crate give a warning.
  - **sources**: crates come only from crates.io and from the git repositories in `deny.toml`. gpui-ce comes in by path (`../gpui-ce`).
- **`npm audit`** in `editors/vscode`. This is the only npm package with third-party packages (all of them are dev tools: `typescript`, `@types/node`, `@types/vscode`). The packages under `npm/` have no third-party dependencies: `npm/cli` has optional dependencies only on its own platform packages (`@plinth/cli-*`). The web host has no npm package. Its one third-party part, the SQLite Wasm build, is a vendored copy (§2, "Web host: the SQLite Wasm build"). `npm audit` does not see it: check the SQLite release notes and advisories by hand when you update it.

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

`plinth-runner-wasmtime` uses the `sys-locale` crate (MIT OR Apache-2.0, from 1Password, about 800 lines) for `plinth:locale` (`docs/I18N.md` §3.1): the OS locale as a BCP 47 tag. On Windows it calls `GetUserDefaultLocaleName` with its own declaration and adds no crate; on Android it adds `libc`. Its feature `js` (for wasm32 in a browser) is off.

`plinth-runner-wasmtime` uses ICU4X 2.3 (Unicode-3.0, from the Unicode Consortium) for the formatting functions of `plinth:locale` (`docs/I18N.md` §3.7): `icu_decimal`, `icu_datetime`, `icu_calendar`, `icu_time`, `icu_plurals`, `icu_list`, `icu_collator`, `icu_locale_core`, `fixed_decimal` and `writeable`, each with its compiled data (`*_data` crates) for all locales, and `icu_experimental` 0.6 for the currency and percent formatters (no stable crate has them yet; its API can change in a minor version). Unicode-3.0 is already in the `allow` list of `deny.toml` (other crates, for example `idna`, already use ICU4X). The new crates in the graph: the ICU4X crates and their data crates, `icu_pattern`, `ixdtf` (Unicode-3.0), `calendrical_calculations` (Apache-2.0), `core_maths` (MIT), `utf16_iter` and `write16` (Apache-2.0 OR MIT). Cost (Windows, 2026-10-08): about 2.5 MB in a release binary (a test binary with all five functions is 2.7 MB, without them 0.25 MB), and about 35 s of build time on a clean release build, in parallel with other crates. The notice file (L1) must include the Unicode license of the data.

### Web host: the SQLite Wasm build

The web host runs `store.sql` (`docs/SQL.md` §2) on the official SQLite Wasm build. It is a vendored copy, not an npm dependency at run time and not a CDN file: the web host has no build step and no npm dependency (`web/README.md`), and `plinth registry serve --web` embeds the web files in the `plinth` binary (`crates/plinth-registry/src/web_files.rs`), so the files must be in the repository.

| Item | Value |
|---|---|
| Package | `@sqlite.org/sqlite-wasm` 3.53.4-build2 on npm (`https://github.com/sqlite/sqlite-wasm`), the build of SQLite 3.53.4 (source id `2026-07-24 19:02:57 bf7c7f30…`), Emscripten 5.0.5 |
| Files | `web/vendor/sqlite-wasm/sqlite3.mjs` (the package file `dist/index.mjs`, 642,742 bytes) and `web/vendor/sqlite-wasm/sqlite3.wasm` (`dist/sqlite3.wasm`, 868,907 bytes). Not changed. |
| SHA-256 | `sqlite3.mjs`: `a6d3fe46aa3f924e7686329f97b0564549ef1f787e0c19924179aef7f72bcc01`; `sqlite3.wasm`: `2ee8f3dab694532afc8840e07703127287662d08b74e6ff50491ce63f00d5752` |
| npm tarball | `sha512-kQVArGJA4z7s56Dm8t+eTBXad4LiohuVj9pfgDNOcxa+Ic5xMrWlLPTq6cT0hw5S4MIvtpsraW+B0wD+IPvC7A==` (with npm provenance) |
| License | SQLite and the SQLite JavaScript code: public domain. The Emscripten glue code in `sqlite3.mjs`: MIT and University of Illinois/NCSA (the header of the file). The npm package: Apache-2.0. All are permissive (§2). |
| Size | About 1.5 MB on disk (0.87 MB Wasm, 0.64 MB JavaScript); the `plinth` binary is that much larger. A page loads it only at the first `store.sql` call of an app. A `.plnt` package does not contain it (the host owns the runtime). |
| Runtime dependencies | None. |

To update it: `npm pack @sqlite.org/sqlite-wasm@<version>`, extract `dist/index.mjs` as `sqlite3.mjs` and `dist/sqlite3.wasm`, write the new version and hashes in this table, run `node web/test/run-sql.mjs`, `cargo test -p plinth-compiler --test sql`, and `node web/test/run-a11y.mjs --hub-only` (Edge, OPFS). Read the SQLite release notes for changes to the authorizer, error texts and the `opfs-sahpool` VFS. The notice file (L1) must include the SQLite Wasm build.

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
| RUSTSEC-2024-0436 (warning only) | `paste` 1.0.15 | No longer maintained. It comes from `pulp` ← `exr` ← `image` ← gpui-ce. A proc macro at build time only. No action until `image` changes. |

## 4. Open items (tracking)

| # | Item | Next step |
|---|---|---|
| A1 | Done (2026-10-27): wasmtime 46.0.3 is now 48.0.7 (it fixes RUSTSEC-2026-0316, RUSTSEC-2026-0327 and the 2026-10 batch). `zip` 2.4 is now 9.1 and `rustls` 0.23.46. | The update needed one small code change (`ZipFile::name()` returns a `Result`). The `ignore` entries are removed from `deny.toml`. |
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
Third-party crates in the checked graph: **794**.

Crates whose license expression names each license (a crate that names two licenses counts once for each):

| License | Crates |
|---|---|
| MIT | 673 |
| Apache-2.0 | 521 |
| Apache-2.0 WITH LLVM-exception | 52 |
| Unicode-3.0 | 47 |
| Zlib | 26 |
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
| Apache-2.0 / MIT | 450 |
| MIT | 159 |
| Unicode-3.0 | 45 |
| Apache-2.0 WITH LLVM-exception | 28 |
| Apache-2.0 / Apache-2.0 WITH LLVM-exception / MIT | 23 |
| Apache-2.0 / MIT / Zlib | 22 |
| Apache-2.0 | 11 |
| BSD-3-Clause | 11 |
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
