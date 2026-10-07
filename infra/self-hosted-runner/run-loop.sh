#!/usr/bin/env bash
# The Linux CI runner of nquandt/plinth: one ephemeral GitHub Actions runner
# registers, runs one job, deregisters, and the loop starts the next one.
# It runs in the WSL2 Ubuntu distro as the systemd service
# plinth-runner.service (see README.md in this directory).
#
# Adapted from anhe-io/drive infra/self-hosted-runner/run-ephemeral-loop.sh,
# which serves the anhe-io org. This one is repo-level (a personal repo
# cannot use org runners) and has its own agent, HOME, work directory and
# CARGO_HOME, so it never shares state with the drive loops. RUSTUP_HOME is
# shared: toolchains are installed once.
#
# Usage: bash run-loop.sh   (reads the token from ~/.plinth-runner-token)
set -euo pipefail

REPO="nquandt/plinth"
AGENT_DIR="${PLINTH_RUNNER_AGENT_DIR:-$HOME/ci-runners/agent-plinth}"
export HOME="${PLINTH_RUNNER_HOME:-$HOME/ci-runners/home-plinth}"
mkdir -p "$HOME"
# On ext4, not /mnt/u: per-file work over 9p is about 100 times slower.
WORK_DIR="$HOME/work"
CACHE_DIR="/mnt/u/ci-runners/cache"
LABELS="self-hosted,Linux,X64,plinth"
NAME="plinth-wsl"

export GH_TOKEN
# tr: the file may come from Windows with CRLF.
GH_TOKEN="$(tr -d '\r\n' < "${PLINTH_RUNNER_TOKEN_FILE:-/home/$(id -un)/.plinth-runner-token}")"
export CARGO_HOME="$CACHE_DIR/cargo-plinth"
export RUSTUP_HOME="$CACHE_DIR/rustup"
# rustup on PATH before a job: dtolnay/rust-toolchain then only calls it.
export PATH="$CARGO_HOME/bin:$PATH"
mkdir -p "$WORK_DIR" "$CARGO_HOME"

fetch_token() {
  gh api -X POST "repos/${REPO}/actions/runners/registration-token" --jq .token
}

cd "$AGENT_DIR"
echo "[plinth-runner] starting in $AGENT_DIR"
trap 'echo "[plinth-runner] stopping"; ./config.sh remove --token "$(fetch_token)" 2>/dev/null || true; exit 0' INT TERM

i=0
while true; do
  i=$((i + 1))
  TOKEN="$(fetch_token)"
  # A hard stop leaves the old registration behind; config.sh then refuses.
  if [ -f .runner ]; then
    ./config.sh remove --token "$TOKEN" || rm -f .runner .credentials .credentials_rsaparams
  fi
  ./config.sh --url "https://github.com/${REPO}" --token "$TOKEN" --unattended --ephemeral \
    --name "${NAME}-${i}" --labels "$LABELS" --work "$WORK_DIR" --replace
  echo "[plinth-runner] waiting for a job (iteration $i)"
  ./run.sh || echo "[plinth-runner] run.sh exited non-zero, registering again"
done
