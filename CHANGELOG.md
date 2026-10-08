# Changelog

This file lists the changes in each Plinth release. Add an entry to "Unreleased" when a change merges that an app author or a user can see. A release moves "Unreleased" under its version number. `docs/PRODUCTION.md` tells what each level (Preview, Beta, 1.0) promises.

## Unreleased

No public release yet. The first release is a Preview for Windows and the web. `docs/LIMITS.md` lists what Preview cannot do.

### Added

- License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
- `docs/PRODUCTION.md`: the production readiness checklist.
- `docs/LIMITS.md`: the known limits.
- `SECURITY.md` and `docs/THREAT-MODEL.md`.

### Changed

- Package format compatibility (`docs/COMPATIBILITY.md` §4): a package with a newer `plinth-profile` (`plinth/2` or higher) gives "This app needs a newer version of Plinth (package format plinth/N)" on desktop and on the web, not an "invalid package" error. A package entry that the host does not know is kept and signed, but never run and never served as an asset (before, the package was refused). `manifest.toml` can have unknown fields at every level, also in `[[capabilities]]` and `[updates]`, and an unknown `[updates]` `channel`, `check` or `apply` value maps to the default. `plinth.toml` stays strict: an unknown field or value there is still an error.

### Security

- `plinth:net`: the `net.local` check now covers all non-public IPv4 and IPv6 ranges (for example `169.254.169.254`, `100.64.0.0/10`, `fd00::/8`, `::ffff:127.0.0.1`) and names such as `localhost.` and `*.localhost`, on both hosts. The desktop host also checks the addresses after DNS resolution, so a public name that resolves to a private address is denied without `net.local`.
- Package signatures: the new schema `plinth.signature/2` also covers `manifest.toml` (its raw bytes), `plinth-profile` and every other entry. Before, a person could change the manifest of a signed package (its `version`, `id`, capabilities, `[updates]` feed or `core` policy) and the signature still verified. Hosts and the web Hub refuse a package signed with `plinth.signature/1`: sign it again with `plinth sign`.
