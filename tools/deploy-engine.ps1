<#
.SYNOPSIS
    Deploys the freshly built Rust engine (AdbTether_core.dll) into the installed
    AdbTether service so the running service picks up the new liveness monitor.

.DESCRIPTION
    The engine runs inside AdbTetherService, which is installed under
    C:\Program Files\AdbTether\service. Replacing AdbTether_core.dll there requires
    elevation. This script stops the service, copies the release DLL, restarts
    the service, and writes a result log next to itself.

.NOTES
    Run elevated:  powershell -ExecutionPolicy Bypass -File deploy-engine.ps1
#>
[CmdletBinding()]
param(
    [string]$Source,
    [string]$Service = 'AdbTetherService',
    [string]$Dest = 'C:\Program Files\AdbTether\service\AdbTether_core.dll'
)

# $PSScriptRoot is not populated while param() defaults are evaluated in
# Windows PowerShell 5.1, so resolve paths here in the body instead.
$here = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
if (-not $Source) { $Source = Join-Path $here '..\target\release\adbtether_core.dll' }
$log = Join-Path $here 'deploy-engine.log'

function Note($msg) {
    $line = "$(Get-Date -Format 'HH:mm:ss') $msg"
    Write-Host $line
    $line | Out-File -FilePath $log -Append -Encoding utf8
}

"=== deploy $(Get-Date -Format o) ===" | Out-File -FilePath $log -Encoding utf8

try {
    $Source = (Resolve-Path $Source).Path
    Note "source : $Source"
    Note "dest   : $Dest"
    Note "source mtime: $((Get-Item $Source).LastWriteTime)"

    Stop-Service $Service -Force -ErrorAction Stop
    Note "service stopped"

    Copy-Item $Source $Dest -Force -ErrorAction Stop
    Note "dll copied"

    Start-Service $Service -ErrorAction Stop
    Note "service started"

    $item = Get-Item $Dest
    Note "installed mtime: $($item.LastWriteTime)  size: $($item.Length)"
    Note "RESULT: OK"
}
catch {
    Note "ERROR: $($_.Exception.Message)"
    Note "RESULT: FAIL"
    exit 1
}
