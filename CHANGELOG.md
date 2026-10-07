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

### Security

- Web host: an app's memory is capped at 64 MiB, the same as the desktop host. A runaway allocation stops the app with "out of memory" instead of using the browser tab's memory.

- `plinth:net`: the `net.local` check now covers all non-public IPv4 and IPv6 ranges (for example `169.254.169.254`, `100.64.0.0/10`, `fd00::/8`, `::ffff:127.0.0.1`) and names such as `localhost.` and `*.localhost`, on both hosts. The desktop host also checks the addresses after DNS resolution, so a public name that resolves to a private address is denied without `net.local`.
