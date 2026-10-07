#!/usr/bin/env bash
# One-time setup of the Linux runner, inside WSL (`wsl -d Ubuntu`). It needs
# no sudo; it prints the two sudo commands at the end.
#   GH_TOKEN=<token with repo scope> bash setup-linux.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
V="${RUNNER_VERSION:-2.338.0}"
agent="$HOME/ci-runners/agent-plinth"
if [ ! -x "$agent/config.sh" ]; then
  mkdir -p "$agent"
  curl -sSL -o /tmp/plinth-runner.tgz "https://github.com/actions/runner/releases/download/v$V/actions-runner-linux-x64-$V.tar.gz"
  tar xzf /tmp/plinth-runner.tgz -C "$agent"
  rm /tmp/plinth-runner.tgz
fi
# The service runs this copy (CRLF removed: the repo may be checked out on Windows).
tr -d '\r' < "$here/run-loop.sh" > "$HOME/ci-runners/plinth-run-loop.sh"
chmod 700 "$HOME/ci-runners/plinth-run-loop.sh"
# The token file: mode 600, never on a command line.
if [ -n "${GH_TOKEN:-}" ]; then
  umask 077
  printf '%s\n' "$GH_TOKEN" > "$HOME/.plinth-runner-token"
fi
mkdir -p /mnt/u/ci-runners/logs
echo "agent: $agent ($(cat "$agent/bin/Runner.Listener.deps.json" >/dev/null 2>&1 && echo ok))"
echo
echo "Now run these two commands (they need your sudo password):"
echo "  sudo apt-get install -y libxkbcommon-x11-dev libxcb-shape0-dev libxcb-xfixes0-dev"
echo "  sudo cp $(wslpath -a "$here" 2>/dev/null || echo "$here")/plinth-runner.service /etc/systemd/system/ && sudo systemctl enable --now plinth-runner"
