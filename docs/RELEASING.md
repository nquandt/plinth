# Releasing Plinth

This page tells you how to cut a release of the `plinth` npm packages.

## Secrets

Set this secret in the GitHub repository settings before you publish:

- `NPM_TOKEN`: an npm automation token with publish rights to the `@plinth`
  org and to `create-plinth`. Store it in the `npm-publish` environment, not
  as a plain repository secret, so you can require a manual approval.

No other secret is needed. The release workflow does not sign binaries yet.

## Normal release

1. Pick a version. Use semantic versioning, for example `0.2.0`.
2. Update the version everywhere:
   ```sh
   node scripts/set-version.mjs 0.2.0
   ```
   This writes the version into `Cargo.toml` and into every `npm/*/package.json`.
3. Commit the version bump:
   ```sh
   git add Cargo.toml npm
   git commit -m "Release 0.2.0"
   ```
4. Tag and push:
   ```sh
   git tag v0.2.0
   git push origin master v0.2.0
   ```
5. The `release.yml` workflow runs on the tag. It:
   - builds the `plinth` binary on Windows, Linux, and macOS (arm64 and x64),
   - packs each platform package and the two meta packages
     (`@plinth/cli`, `create-plinth`),
   - publishes all of them to npm with `--provenance`.
6. Check the Actions tab. The publish job needs the `npm-publish`
   environment approval if you added one.

## Manual or partial release

- To build and pack without publishing, run the workflow with
  `workflow_dispatch` and leave "Publish to npm" unchecked (or push without
  a tag; `ci.yml` never publishes).
- To publish from an existing build, run `workflow_dispatch` on `release.yml`
  with "Publish to npm" checked. This still needs `NPM_TOKEN`.

## Checking a build locally

You cannot build every platform locally, but you can pack the package you
*can* build and check versions:

```sh
cargo build --release -p plinth-cli
node scripts/set-version.mjs
node scripts/npm-pack.mjs
```

This writes tarballs to `target/npm/`. Install one in a scratch directory
with `npm install /path/to/plinth-cli-*.tgz` to check it runs.

## Local npm smoke test

This is the full end-to-end flow a new user goes through, checked locally
before a release. It needs a release build and the tarballs:

```sh
cargo build --release -p plinth-cli
node scripts/npm-pack.mjs
```

Then, in a scratch directory under `target/` (outside the repo's own
`node_modules`):

```sh
mkdir target/npm-smoke && cd target/npm-smoke
cp ../npm/*.tgz .
```

**Gotcha:** `npx <tarball>` alone runs nothing useful on Windows; use
`npx --package=<tarball> <bin-name>`. `create-plinth`'s first run also has
no `@plinth/cli` installed yet to find the `plinth` binary, and it is not
on the npm registry during a local smoke test, so point it at the release
binary directly with `PLINTH_BINARY`:

```sh
PLINTH_BINARY=../../release/plinth.exe npx --yes --package=./create-plinth-0.1.0.tgz create-plinth my-app
cd my-app
npm install ../plinth-cli-0.1.0.tgz ../plinth-cli-win32-x64-0.1.0.tgz
npm run check
npm run build
```

(On Windows, `PLINTH_BINARY` can also be set with PowerShell's
`$env:PLINTH_BINARY = "..."` before the `npx` line, or inline for bash as
shown.) `npm install` adds `node_modules/.bin/plinth{,.cmd,.ps1}`, wired
through `@plinth/cli`'s `bin/plinth.js` shim to the platform package's
binary; `npm run check` and `npm run build` then call that shim with no
further setup. The project's typings live at `.plinth/types` (written by
`plinth new` and refreshed by `plinth check`/`build`); `tsconfig.json`
points `plinth:*` there, so `npx -p typescript@7 tsc -p .` also works with
no changes.

This flow was last verified on Windows against version 0.1.0 with no
shim, script, or typings-path changes needed: `create-plinth` scaffolds a
working project, `npm install` resolves the two tarballs, and `npm run
check` / `npm run build` both succeed against the generated two-screen
template.

## Updating the gpui-ce pin

CI checks out `gpui-ce` next to `plinth` at a pinned commit (see
`GPUI_CE_COMMIT` near the top of `ci.yml` and `release.yml`). When the
user's `gpui-ce` fork moves forward and the workspace builds against the
new commit locally:

1. Run `git -C ../gpui-ce rev-parse HEAD` to get the new commit.
2. Update `GPUI_CE_COMMIT` in both workflow files to that commit.
3. Commit the change together with whatever plinth change needed it.

## What is not released yet

- Code signing / notarization for the macOS and Windows binaries.
- A changelog step. Write release notes by hand on the GitHub release for
  now.
- Linux and macOS are built but not tested by CI (see `ci.yml`); watch the
  first few releases on those platforms by hand.
