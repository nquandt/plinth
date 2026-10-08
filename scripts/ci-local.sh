#!/usr/bin/env bash
# The CI checks, in one script that CI and developers both run.
#
#   bash scripts/ci-local.sh full    # everything (Windows CI; run this before every push)
#   bash scripts/ci-local.sh nogui   # no GUI host tests (Linux and macOS CI)
#   bash scripts/ci-local.sh full --shoot   # also the headless screenshot tests (needs a GPU; local only)
#   bash scripts/ci-local.sh deny    # only the dependency audit (Linux CI; also part of full)
#
# Run it from the repository root. It stops at the first failure.
# Test crates run one at a time: on Windows, linking several large test
# binaries at once fails (LNK1318/LNK1201).
set -euo pipefail

mode="${1:-full}"
shoot=no
[ "${2:-}" = "--shoot" ] && shoot=yes
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}"

step() { echo; echo "::group::$*"; }
end() { echo "::endgroup::"; }

# Crates whose tests need no window and no GPU.
common=(plinth-protocol plinth-package plinth-link plinth-compiler plinth-rt plinth-runner-wasmtime plinth-hub plinth-registry)
# Crates that build the GUI host (gpui-ce); their tests open no window.
gui=(plinth-ui plinth-host-desktop plinth-cli)

# Dependency audit (deny.toml, docs/DEPENDENCIES.md): cargo deny (advisories,
# licenses, bans, sources) and npm audit. An unmaintained crate is a warning
# (-W unmaintained); the gpui-ce skips in deny.toml are on purpose
# (-A unnecessary-skip). A machine without cargo-deny or npm skips that part,
# so the rest of the gate runs.
deny_check() {
  step "dependency audit (cargo deny, npm audit)"
  if cargo deny --version > /dev/null 2>&1; then
    cargo deny check -W unmaintained -A unnecessary-skip
  else
    echo "SKIPPED: cargo-deny is not installed (cargo install cargo-deny --locked)."
  fi
  # npm: only editors/vscode has a lockfile and third-party packages (all of
  # them dev tools). The packages under npm/ have no third-party dependencies.
  if command -v npm > /dev/null 2>&1; then
    echo "npm audit editors/vscode"
    (cd editors/vscode && npm audit --audit-level=moderate)
  else
    echo "SKIPPED: npm audit (npm is not installed)."
  fi
  end
}

case "$mode" in
  full)
    deny_check
    crates=("${common[@]}" "${gui[@]}")
    ;;
  deny)
    deny_check
    echo
    echo "ci-local: dependency audit passed"
    exit 0
    ;;
  nogui)
    step "cargo check (workspace)"
    cargo check --workspace
    end
    crates=("${common[@]}")
    ;;
  *) echo "usage: $0 full|nogui|deny [--shoot]" >&2; exit 2 ;;
esac

for crate in "${crates[@]}"; do
  step "cargo test -p $crate"
  cargo test -p "$crate"
  end
done

if [ "$mode" = full ]; then
  step "tsc check of every example"
  for dir in examples/*/ examples/7guis/*/; do
    [ -f "$dir/tsconfig.json" ] || continue
    echo "tsc $dir"
    (cd "$dir" && npx -y -p typescript@7 tsc -p .)
  done
  end

  step "web host tests (node)"
  cargo build -p plinth-cli
  plinth=./target/debug/plinth
  "$plinth" core export target/core.wasm
  # Build every Plinth project that a web test can read, so a new test never
  # depends on a list that someone must remember to update.
  for dir in examples/*/ examples/7guis/*/ web/test/fixtures/*/; do
    [ -f "$dir/plinth.toml" ] || continue
    "$plinth" build "$dir" > /dev/null
  done
  for t in web/test/run-*.mjs; do
    # run-a11y.mjs needs a local Edge and the axe-core CDN; run it by hand.
    [ "$t" = web/test/run-a11y.mjs ] && continue
    echo "node $t"
    node "$t"
  done
  end
fi

if [ "$shoot" = yes ]; then
  step "cargo test -p plinth-shoot (headless GPU)"
  cargo test -p plinth-shoot
  end
fi

echo
echo "ci-local: all checks passed ($mode)"
