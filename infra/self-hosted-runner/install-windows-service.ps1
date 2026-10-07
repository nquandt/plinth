# Installs the Windows CI runner of nquandt/plinth as a Windows service
# (README.md in this directory). Run it one time in an ELEVATED PowerShell:
#
#   powershell -ExecutionPolicy Bypass -File C:\repos\plinth\infra\self-hosted-runner\install-windows-service.ps1
#
# The service runs as your own account, so jobs use your Rust toolchain,
# Node and Git, and the build output stays warm between runs. It asks for
# your Windows password (the service logon needs it). The registration
# token comes from your `gh` login and is used at once; it is not stored.
#
# Run it again to replace the registration (for example after a reinstall).
$ErrorActionPreference = 'Stop'

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this script in an elevated (Administrator) PowerShell.'
}

$repo = 'nquandt/plinth'
$dir = 'U:\ci-runners\plinth-win'
$name = 'plinth-win'
if (-not (Test-Path "$dir\config.cmd")) {
    throw "The runner agent is not in $dir. Extract actions-runner-win-x64 there first (README.md)."
}
Set-Location $dir

$token = gh api -X POST "repos/$repo/actions/runners/registration-token" --jq .token
if (-not $token) { throw 'gh could not get a registration token (gh auth status: needs the repo scope).' }

if (Test-Path "$dir\.runner") {
    Write-Host 'Removing the old registration...'
    & .\config.cmd remove --token $token
}

$account = "$env:USERDOMAIN\$env:USERNAME"
$secure = Read-Host "Windows password for $account (for the service logon)" -AsSecureString
$bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
try {
    $password = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
    & .\config.cmd --unattended --url "https://github.com/$repo" --token $token --name $name `
        --labels plinth --work _work --replace `
        --runasservice --windowslogonaccount $account --windowslogonpassword $password
    if ($LASTEXITCODE -ne 0) { throw "config.cmd failed ($LASTEXITCODE)" }
} finally {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    $password = $null
}

# The jobs run steps with `shell: bash`: put Git Bash on the runner's PATH
# (the machine PATH has only Git\cmd). The runner reads .path at start.
$path = @(
    [Environment]::GetEnvironmentVariable('Path', 'Machine'),
    [Environment]::GetEnvironmentVariable('Path', 'User'),
    'C:\Program Files\Git\bin',
    'C:\Program Files\Git\usr\bin'
) -join ';'
Set-Content -Path "$dir\.path" -Value $path -Encoding ascii -NoNewline

$service = Get-Service | Where-Object { $_.Name -like "actions.runner.*$name*" } | Select-Object -First 1
if ($service) {
    Restart-Service $service.Name
    Write-Host "Service $($service.Name): $((Get-Service $service.Name).Status)"
}
Write-Host 'Done. Check: gh api repos/nquandt/plinth/actions/runners --jq ".runners[] | [.name, .status] | @tsv"'
