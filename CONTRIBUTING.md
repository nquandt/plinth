# Contributing to Plinth

Thank you for your interest in Plinth. This page covers building and
testing the project, a few conventions worth knowing before you send a
change, and where to start.

## Prerequisites

- Rust (stable), with the Wasm target: `rustup target add wasm32-unknown-unknown`
- [`gpui-ce`](https://github.com/nquandt/gpui-ce) checked out next to
  this repository, so it resolves as `../gpui-ce`
- Node 22+ (for the npm scripts and the `tsc` checks on the examples)
- **Windows** is the only platform this project currently tests. Linux
  and macOS builds exist in CI but are best effort; please say so in
  your PR if you cannot verify them.

## Building

```sh
cargo build -p plinth-cli
```

builds `target/debug/plinth.exe`, the tool used throughout this repo
and its docs. For a smaller, faster binary (and the one `plinth native`
embeds), use `--release`.

## Testing

**Run test crates one at a time on Windows.** The gpui-ce and wasmtime
builds are large, and running several `cargo test` invocations across
crates at once can produce linker errors (`LNK1318`, `LNK1201`). If you
hit one, lower `CARGO_BUILD_JOBS` (3–6 is usually enough) and keep
running crates individually:

```sh
cargo test -p plinth-compiler      # front end, link, and e2e tests of the examples
cargo test -p plinth-rt
cargo test -p plinth-protocol
cargo test -p plinth-ui
cargo test -p plinth-package
cargo test -p plinth-runner-wasmtime
cargo test -p plinth-host-desktop
```

The examples under `examples/` are meant to be valid, ordinary
TypeScript too; check one with:

```sh
cd examples/todo && npx -y -p typescript@7 tsc -p .
```

It should print nothing.

## Repository conventions

- **Commit after each tested step.** Small, verified commits are much
  easier to review and to bisect than one large change.
- **Never GPG-sign commits**, and do not add attribution lines unless a
  project convention asks for them.
- `SPEC.md` is the design document, written in ASD-STE100 Simplified
  Technical English (short sentences, active voice, one idea per
  sentence). `HANDOFF.md` is a living state-of-the-project note for
  whoever picks up work next. Keep both documents' style and intent
  when you touch nearby prose, and check the diff carefully before you
  change either — they are the project's source of truth for design
  decisions already made.
- **Worktrees for parallel work:** if you use a git worktree to work on
  Plinth alongside other changes, create it as a sibling directory
  (`../plinth-wt-<name>`) so `../gpui-ce` still resolves, and copy
  `target/debug` and `target/rt` into it first to reuse the build
  cache.
- **The capability map, the UI API id table, and the runtime ABI are
  append-only contracts.** Never renumber an id in
  `wit/plinth/ui-api.toml`, and never remove, retype, or change the
  behavior of a function in `crates/plinth-rt`'s exported ABI inside a
  major core version — add a new function and bump the minor version
  instead (SPEC.md §10.5).

## Where to start

[HANDOFF.md](HANDOFF.md) is the single best entry point: it describes
what exists, what is in progress, the gotchas people have already hit,
and a suggested order of next steps (currently: the Plinth Hub's
local-library phase, a web host spike, async host calls, and more UI
controls). Read it, then [SPEC.md](SPEC.md) for the design and this
repository's docs under `docs/` for topic-by-topic references.

## License

Plinth is intended to be released under the Apache License 2.0 (see
the `license` field in `Cargo.toml`). A `LICENSE` file has not been
added to the repository yet — that is the project owner's decision to
make before the first public release. Please do not add one in a
contribution; flag it to a maintainer instead.

## Before you push

Run the same checks that CI runs, and push only when they pass:

```sh
bash scripts/ci-local.sh full            # every test crate, tsc of every example, the web host tests
bash scripts/ci-local.sh full --shoot    # also the headless screenshot tests (needs a GPU)
```

CI (`.github/workflows/ci.yml`) runs this script, so a green local run means a green CI run, except for platform differences. Batch your commits into few pushes: each push to `master` starts a Windows and a Linux run. macOS runs only once a week or by hand, because its minutes cost ten times more.

