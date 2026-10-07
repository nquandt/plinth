# Sets up the desktop Hub for local development (examples/hub/README.md "Run it").
#
#   powershell -File scripts/dev-hub.ps1            # set up, then start the Hub
#   powershell -File scripts/dev-hub.ps1 -NoRun     # set up only
#
# It uses `plinth` from the PATH (`cargo install --path crates/plinth-cli`).
# It does not change any tracked file: it copies examples/hub to
# target/dev-hub, sets `publisher` there to the name of your publisher key,
# then builds and signs that copy. Run it again after a change to the Hub app.
#
# Your publisher key must exist (`plinth publisher init --name "<name>"`).
# PLINTH_HUB_TRUSTED_KEYS must contain your key id, for this session or as a
# user environment variable.
param([switch]$NoRun)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

# "Nate  ed25519:<key id>" -> name and key id.
$show = (& plinth publisher show) -join "`n"
if ($LASTEXITCODE -ne 0) { throw "no publisher key: run plinth publisher init --name ""<name>""" }
if ($show -notmatch '^(?<name>.+?)\s+(?<key>ed25519:\S+)') { throw "unexpected output of plinth publisher show: $show" }
$name = $Matches.name.Trim()
$key = $Matches.key
if (-not (($env:PLINTH_HUB_TRUSTED_KEYS -split ',') -contains $key)) {
  Write-Host "PLINTH_HUB_TRUSTED_KEYS does not contain $key; adding it for this session."
  $env:PLINTH_HUB_TRUSTED_KEYS = (@($env:PLINTH_HUB_TRUSTED_KEYS, $key) | Where-Object { $_ }) -join ','
}

# A copy of the Hub app with your publisher name.
$dev = "target/dev-hub"
if (Test-Path $dev) { Remove-Item -Recurse -Force $dev }
New-Item -ItemType Directory -Force $dev | Out-Null
Copy-Item -Recurse examples/hub/app, examples/hub/tsconfig.json, examples/hub/plinth.toml $dev
# UTF-8 without a BOM; Windows PowerShell 5.1 would otherwise read the file as ANSI.
$tomlPath = (Resolve-Path "$dev/plinth.toml").Path
$toml = [IO.File]::ReadAllText($tomlPath, [Text.Encoding]::UTF8)
$toml = $toml -replace '(?m)^publisher\s*=\s*".*"', ('publisher = "' + $name + '"')
[IO.File]::WriteAllText($tomlPath, $toml, (New-Object Text.UTF8Encoding $false))

& plinth build $dev --sign
if ($LASTEXITCODE -ne 0) { throw "build failed" }
$hub = Get-ChildItem "$dev/dist/*.plnt" | Select-Object -First 1
& plinth validate $hub.FullName
if ($LASTEXITCODE -ne 0) { throw "validate failed" }

# The library: the Hub app (granted hub.manage) and some examples.
& plinth hub add $hub.FullName
& plinth hub grants dev.plinth.hub allow hub.manage
foreach ($app in "notes", "budget", "todo", "counter", "timer", "calculator", "pong", "primitives", "files-demo") {
  & plinth build "examples/$app" | Out-Null
  $pkg = Get-ChildItem "examples/$app/dist/*.plnt" | Select-Object -First 1
  if ($pkg) { & plinth hub add $pkg.FullName }
}

# A source for the Discover screen: the web Hub demo registry, if it exists.
if (Test-Path target/web-hub-registry/apps/index.json) {
  & plinth hub source add local (Resolve-Path target/web-hub-registry).Path 2>$null
}

if (-not $NoRun) { & plinth hub ui }
