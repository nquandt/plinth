# Changelog

This file lists the changes in each Plinth release. Add an entry to "Unreleased" when a change merges that an app author or a user can see. A release moves "Unreleased" under its version number. `docs/PRODUCTION.md` tells what each level (Preview, Beta, 1.0) promises.

## Unreleased

No public release yet. The first release is a Preview for Windows and the web. `docs/LIMITS.md` lists what Preview cannot do.

### Added

- License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
- `docs/PRODUCTION.md`: the production readiness checklist.
- `docs/LIMITS.md`: the known limits.
- `SECURITY.md` and `docs/THREAT-MODEL.md`.
- `docs/COMPATIBILITY.md`: the public contracts, their version rules and the deprecation policy.
- `docs/PRIVACY.md`, `CODE_OF_CONDUCT.md`, and a pull request guide in `CONTRIBUTING.md`.
- `docs/host-apis.md` "Support on each host": how each host API works on the desktop (wasmtime and wasmi) and the web, and the test that covers it on each host.
- `docs/guides/`: guides for app authors (storage, network, updates, publishing, accessibility, performance), each with a checked example.
- Desktop self-update: an update that adds a Medium or High capability shows the consent window for the new capabilities before it runs. If the user cancels, the previous version keeps running, and the host does not ask again for that version.
- Desktop self-update: `apply = "prompt"` shows "An update is ready. Restart now?" with "Restart" and "Later" in the app window when an update is ready.
- The `plinth-host` executable now checks for updates when it runs an exported app, the same as the `plinth` executable.

### Changed

- Package format compatibility (`docs/COMPATIBILITY.md` §4): a package with a newer `plinth-profile` (`plinth/2` or higher) gives "This app needs a newer version of Plinth (package format plinth/N)" on desktop and on the web, not an "invalid package" error. A package entry that the host does not know is kept and signed, but never run and never served as an asset (before, the package was refused). `manifest.toml` can have unknown fields at every level, also in `[[capabilities]]` and `[updates]`, and an unknown `[updates]` `channel`, `check` or `apply` value maps to the default. `plinth.toml` stays strict: an unknown field or value there is still an error.

- Regular expressions: literals, `new RegExp`, `test`, `exec`, `lastIndex`, the flags `g i m s`, with a linear-time matcher in the core (core 1.12). `docs/language.md` "Regular expressions" lists the subset.

### Security

- Web host: an app's memory is capped at 64 MiB, the same as the desktop host. A runaway allocation stops the app with "out of memory" instead of using the browser tab's memory.
- `plinth:net`: the `net.local` check now covers all non-public IPv4 and IPv6 ranges (for example `169.254.169.254`, `100.64.0.0/10`, `fd00::/8`, `::ffff:127.0.0.1`) and names such as `localhost.` and `*.localhost`, on both hosts. The desktop host also checks the addresses after DNS resolution, so a public name that resolves to a private address is denied without `net.local`.
- Package signatures: the new schema `plinth.signature/2` also covers `manifest.toml` (its raw bytes), `plinth-profile` and every other entry. Before, a person could change the manifest of a signed package (its `version`, `id`, capabilities, `[updates]` feed or `core` policy) and the signature still verified. Hosts and the web Hub refuse a package signed with `plinth.signature/1`: sign it again with `plinth sign`.
