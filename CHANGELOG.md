# Changelog

This file lists the changes in each Plinth release. Add an entry to "Unreleased" when a change merges that an app author or a user can see. A release moves "Unreleased" under its version number. `docs/PRODUCTION.md` tells what each level (Preview, Beta, 1.0) promises.

## Unreleased

No public release yet. The first release is a Preview for Windows and the web. `docs/LIMITS.md` lists what Preview cannot do.

### Added

- License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
- `docs/PRODUCTION.md`: the production readiness checklist.
- `docs/LIMITS.md`: the known limits.
- Desktop self-update: an update that adds a Medium or High capability shows the consent window for the new capabilities before it runs. If the user cancels, the previous version keeps running, and the host does not ask again for that version.
- Desktop self-update: `apply = "prompt"` shows "An update is ready. Restart now?" with "Restart" and "Later" in the app window when an update is ready.
- The `plinth-host` executable now checks for updates when it runs an exported app, the same as the `plinth` executable.
