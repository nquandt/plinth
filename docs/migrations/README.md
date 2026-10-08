# Migration guides

This folder has one guide for each core minor version and one guide for each breaking change (`docs/PRODUCTION.md` X5). A guide tells each reader what to do:

- **App authors:** what is new, and what changes when you build with the new version.
- **Host authors:** what a host must supply before it uses the new core (`docs/COMPATIBILITY.md` §3).
- **Users:** what they see, if anything.

The rules for what can change in a minor version are in `docs/COMPATIBILITY.md`. Inside major version 1, a minor version only adds. An app that was built one time keeps running.

## Guides

| Version | Kind | Status | Guide |
|---|---|---|---|
| Core 1.13 | Minor: the SQL store | Not shipped yet | [core-1.13.md](core-1.13.md) |

Core 1.12 and older have no guide. They were made before the first public release, and the first public release ships them together.

## How to write a guide

1. Make the guide in the same change that starts the new minor version (raises `rt_abi::CORE_MINOR`).
2. Add each new core function, host interface and `plinth:*` export to the guide in the change that adds it.
3. When the minor version ships (`rt_abi::SHIPPED_MINOR`), change its status to "Shipped" with the date, and add a link from `CHANGELOG.md`.
4. A breaking change (a new major version) needs a guide with the steps to move an app, a table of each removed item and its replacement, and the date when the old version stops getting fixes.
