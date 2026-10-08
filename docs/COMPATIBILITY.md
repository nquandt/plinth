# Compatibility rules

This document lists every public contract of Plinth, the version rule of each contract, and how a change is deprecated. It covers checklist items C4, C5, C6 and C7 in `docs/PRODUCTION.md`. The basic promise: **an app that was built one time keeps running on newer hosts.** Plinth has rolling support (`docs/PRODUCTION.md` §15), so this promise comes from the rules below, not from a support window.

Status of this file: first version, 2026-10-07.

## 1. The contracts

| Contract | Version | Where | Who must stay compatible |
|---|---|---|---|
| UI API (controls, props, events, enums) | `MAJOR.MINOR` in `wit/plinth/ui-api.toml` | SPEC §6.6 | Hosts with old apps |
| Core ABI (runtime functions, object layout) | `MAJOR.MINOR.PATCH`, `rt_abi.rs` | SPEC §10.5 | Cores with old apps |
| Host interfaces (`wit/plinth/app.wit`) | `plinth:app@1.0.0`, plus the core minor that added each item | SPEC §8.1, §3 below | Hosts with old cores |
| `.plnt` container | `plinth-profile` (`plinth/1`) | SPEC §10.1, §4 below | Hosts with old packages |
| Manifest (`manifest.toml`) | Part of `plinth/1` | SPEC §10.2, §4 below | Hosts with old and new packages |
| Signature (`signature.json`) | `schema = "plinth.signature/2"` | `docs/HUB.md` §6.1 | Hosts and the Hub |
| Update feed (`feed.json`) | Its `schema` field | `docs/UPDATES.md` §7 | Updaters with old and new feeds |
| Plinth TS (the language and `plinth:*` modules) | The compiler version | SPEC §4, §5 below | The compiler with old sources |
| Project files (`plinth.toml`, CLI commands) | The CLI version | SPEC §13 | The CLI with old projects |

## 2. General rules

1. **Add, do not change.** Inside one major version, a contract only gets new items. An existing item keeps its name, its id, its type and its behavior.
2. **A new item is optional.** An old producer does not know it. So a new manifest field, a new prop or a new feed field has a default that gives the old behavior.
3. **Read old, tolerate new.** A consumer reads every older version of its input. It ignores a field that it does not know, unless the field changes the meaning of the input; such a change needs a new major version (a new profile or schema).
4. **A break needs a new major version, and the old one stays.** Hosts keep the old core and read the old format side by side with the new one (SPEC §10.5, `docs/PRODUCTION.md` C10).
5. **A test guards each rule.** §7 lists the checks.

## 3. Host interfaces (C5)

`wit/plinth/app.wit` is the interface between a core and the host. A core is a file that can be years old; a host must link it.

- The package stays `plinth:app@1.0.0` for all of major version 1. The import names of a core contain this version, so a change of it breaks every old core.
- Each new function, interface, type or enum case comes with a new core minor version, and `rt_abi::ADDED_IN` records which minor added the core functions that use it (HANDOFF §6).
- Do not change the parameters, the results or the meaning of an existing function. Do not add a field to an existing record, or a case to an existing variant or enum that an old core reads: in the component model, that changes the type. Add a new function with the new type instead (for example `fetch2`).
- A host supplies every function of every interface that any supported core imports. A capability that a host cannot give answers `denied(unsupported)`; it does not leave the import out.
- Removal is only possible in a new major version (`plinth:app@2.0.0`).

## 4. The `.plnt` container and the manifest (C4)

- `plinth-profile` names the container version. Today it is `plinth/1`. A host reads every profile up to the newest that it knows. A host that gets a newer profile says "This app needs a newer version of Plinth", not "invalid package".
- Inside `plinth/1`:
  - The entries (`manifest.toml`, the entry module, `assets/`, `signature.json`, `source/`) keep their meaning. A new optional entry is allowed; a reader ignores an entry that it does not know, but never runs it and never serves it as an asset. The signature (`plinth.signature/2`) covers it, as it covers every entry.
  - A new manifest field is optional and has a default that gives the old behavior (example: `core`, default `follow`; `updates`, default none).
  - A host ignores a manifest field that it does not know, at every level of the manifest (also inside `[[capabilities]]` and `[updates]`). The author's `plinth.toml` stays strict: the CLI rejects an unknown field there, because it is most likely a typing error.
  - An enum value that a host does not know (for example a new `apply` policy) maps to the safe default of that field, and the host logs it.
- The zip limits (entry size, total size, entry count) can only grow inside `plinth/1`.

**Status:** done.

- Profile: `Package::read` reads `plinth-profile` first. `plinth/1` up to `plinth_package::PROFILE_VERSION` is accepted. A larger number gives the error `NewerFormat`, "This app needs a newer version of Plinth (package format plinth/N)"; callers do not add "not a valid package" to it (`plinth_package::read_error`). Any other text is an error. The web host does the same check (`checkPlntProfile` in `web/zip.js`, used by `readPlnt` and the web Hub).
- Entries: `Package::read` keeps an unknown entry in `Package::other_entries`, with `source/…`. The host never serves it and never runs it. `write` writes it again, and the signature covers it.
- Manifest: `Manifest`, `Capability` and `UpdatesConfig` ignore unknown fields. The enums of `[updates]` (`channel`, `check`, `apply`) map an unknown value to the default of the field and log a warning (`log::warn!`). `ProjectConfig::parse` keeps `plinth.toml` strict: it denies an unknown top-level field, checks the fields of `[[capabilities]]` and `[updates]` and each enum value (`check_strict`), and `core` stays a strict enum.
- Tests: `plinth-package` (`a_manifest_with_unknown_fields_and_enum_values_reads`, `plinth_toml_stays_strict`, `an_unknown_entry_is_kept_but_never_an_asset`, `a_newer_profile_needs_a_newer_plinth`, `signature::tests::unknown_entries_are_signed`), `plinth-hub` (`a_package_in_a_newer_format_needs_a_newer_plinth`) and `web/test/run-hub-integrity.mjs` (a newer profile, an unknown entry signed by `plinth sign`).

## 5. Plinth TS (C6)

- The language version is the version of the compiler (`plinth --version`). The accepted syntax is in SPEC §4.3 and `docs/language.md`; the deviations from ECMAScript are in SPEC §4.6.
- **A source that compiles stays compiling** inside one major version, with the same behavior. A new feature turns an error into a working program; it never turns a working program into an error.
- A program that was accepted by mistake (a compiler defect) can get an error in a minor version only if it was unsafe (for example, a wrong type at run time). Otherwise it gets a warning first (§6).
- `plinth:*` modules follow the same rule as the core ABI: a new export is a new function; an existing export keeps its signature.
- The 1.0 language is the set of features in `docs/language.md` at the 1.0 release.

## 6. Deprecation (C7)

A deprecation tells authors that an item goes away in the next major version. It never stops an existing app.

1. **Mark it.** Write "Deprecated in X.Y: use Z" in the reference doc (`docs/ui.md`, `docs/host-apis.md`, `docs/language.md`), in the typings (`/** @deprecated use Z */` in `std/*.d.ts`, so the editor shows it), and in `CHANGELOG.md`.
2. **Warn at build time.** The compiler gives a warning with its own code (a `PLW` code, `Severity::Warning`) and a help line that names the replacement. A warning never fails `plinth build`, unless the author asks for it (`--deny-warnings`, planned).
3. **Keep it working.** A deprecated item works in every release of the same major version. Hosts and cores keep it.
4. **Remove it only in a new major version,** with a migration guide (`docs/PRODUCTION.md` X5). Old apps keep running on the old core and the old host interfaces, which stay available (rule 4 in §2).
5. **Minimum time:** a deprecation is in at least one released minor version before the next major version.

## 7. Checks

| Rule | Check | Status |
|---|---|---|
| UI API ids are append-only | Version assert in `plinth-protocol`; generated id files | Done |
| Core functions are append-only | The linker checks names and types; `ADDED_IN` | Done |
| Old apps link with the newest core | `tests/e2e.rs` and the app tests link the examples with the newest core | Partial: no frozen old packages yet (C8) |
| `app.wit` is append-only | `wit/plinth/app.wit.snapshot` and `plinth-link/tests/wit_append_only.rs`: a changed or removed item fails; a new item fails until it is blessed (`PLINTH_BLESS=1`) | Done |
| Hosts read old packages | Frozen `.plnt` files of each released version, read and run in the gate | Open (C8) |
| Hosts tolerate new manifest fields | A test that adds unknown fields at every level of a manifest, and unknown enum values; `plinth.toml` with the same is rejected | Done (`plinth-package` tests, §4) |
| Hosts keep unknown entries | A package with an unknown entry reads, is signed and verifies; the entry is never an asset | Done (`plinth-package` tests, `run-hub-integrity.mjs`) |
| A newer profile says "needs a newer Plinth" | `plinth/2` gives `NewerFormat`; a profile that is not valid stays an error | Done (`plinth-package`, `plinth-hub`, `run-hub-integrity.mjs`) |
| Deprecation warnings | A golden test for each `PLW` code | Open (no deprecation yet) |
