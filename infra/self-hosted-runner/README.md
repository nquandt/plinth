# Self-hosted CI runners for nquandt/plinth

The GitHub Actions minutes of this private repo ran out (2026-10-06). The CI jobs for Windows and Linux now run on the owner's PC, and they use no minutes. Only the weekly macOS job and `release.yml` still use GitHub-hosted runners.

| Job | Runner | Where | Mode |
|---|---|---|---|
| Windows (full) | `plinth-win`, labels `self-hosted, Windows, X64, plinth` | `U:\ci-runners\plinth-win` | A Windows service that runs as the owner's account. Persistent: `target/` stays warm between runs (`clean: false` in `ci.yml`). |
| Linux (no GUI) | `plinth-wsl-<n>`, labels `self-hosted, Linux, X64, plinth` | WSL2 `Ubuntu`: agent `~/ci-runners/agent-plinth`, HOME and work `~/ci-runners/home-plinth` | Ephemeral: one job per registration, in a loop (`run-loop.sh`), as the systemd service `plinth-runner`. |

The Linux runner follows the setup of the `anhe-io` runners (`C:\repos\drive\infra\self-hosted-runner`), but it is a repo-level runner: a personal repo cannot use the runners of an org. It has its own agent, HOME, work directory and `CARGO_HOME` (`/mnt/u/ci-runners/cache/cargo-plinth`). It shares only `RUSTUP_HOME` (`/mnt/u/ci-runners/cache/rustup`), so it never changes the state of the `anhe-io` loops. It also uses their keepalive: the scheduled task `WSL Ubuntu keepalive (ci-runners)` keeps the distro running, so the service starts at logon.

The repo is private, so only the owner can start jobs on these runners. Do not make the repo public while they are registered: a pull request from a fork could then run code on this PC.

## One-time setup

The agents are already downloaded (runner 2.338.0, 2026-10-07). Do these steps one time.

### Windows

In an **elevated** PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File C:\repos\plinth\infra\self-hosted-runner\install-windows-service.ps1
```

The script asks for your Windows password: the service logs on as your account, so jobs use your Rust toolchain, Node and Git. It gets a registration token from your `gh` login, registers `plinth-win`, installs the service, and gives the runner a PATH with Git Bash (`C:\Program Files\Git\bin`).

To download the agent again: extract `actions-runner-win-x64-<version>.zip` from <https://github.com/actions/runner/releases> into `U:\ci-runners\plinth-win`.

### Linux (WSL)

1. Give the loop a token with the `repo` scope. In PowerShell (not elevated):

   ```powershell
   gh auth token | wsl -d Ubuntu -- bash -c 'umask 077; cat > ~/.plinth-runner-token'
   ```

   The loop reads this file (mode 600) and gets a new registration token for each job.

2. Inside WSL (`wsl -d Ubuntu`), install the packages that gpui-ce needs and the service. These commands need your sudo password:

   ```bash
   sudo apt-get install -y libxkbcommon-x11-dev libxcb-shape0-dev libxcb-xfixes0-dev
   sudo cp /mnt/c/repos/plinth/infra/self-hosted-runner/plinth-runner.service /etc/systemd/system/
   sudo systemctl enable --now plinth-runner
   ```

   (`libxkbcommon-dev`, `libwayland-dev`, `libx11-dev` and `libfontconfig-dev` were already installed.)

`setup-linux.sh` downloads the agent and copies `run-loop.sh` to `~/ci-runners/plinth-run-loop.sh`, which the service runs. Run it again after a change to `run-loop.sh`:

```bash
bash /mnt/c/repos/plinth/infra/self-hosted-runner/setup-linux.sh
```

## Check and operate

```powershell
gh api repos/nquandt/plinth/actions/runners --jq '.runners[] | [.name, .status, .busy] | @tsv'
Get-Service actions.runner.*plinth*
```

```bash
systemctl status plinth-runner             # inside WSL
tail -f /mnt/u/ci-runners/logs/loop-plinth.log
sudo systemctl restart plinth-runner
```

- If a job waits in `queued`, a runner is offline. Check the commands above.
- If `C:` fills, WSL fails (see "Recovery after a crash" in the `anhe-io` README). The Windows runner keeps its work on `U:` for this reason.
- The Windows job and local builds share `%USERPROFILE%\.cargo` (cargo locks it), and they compete for CPU. `CARGO_BUILD_JOBS` is 3 in `ci.yml`.

## Turning this off

Stop the runners (`Stop-Service actions.runner.*plinth*`; `sudo systemctl disable --now plinth-runner`) and set `runs-on:` in `.github/workflows/ci.yml` back to `windows-latest` and `ubuntu-latest`. On a hosted Linux runner, add the `apt-get install` step back. To remove a registration: `U:\ci-runners\plinth-win\config.cmd remove --token <token>` (a token from `gh api -X POST repos/nquandt/plinth/actions/runners/remove-token --jq .token`).
