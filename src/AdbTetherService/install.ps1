# Installs AdbTetherService as a Windows service (SYSTEM account).
# Requires admin. Usage:
#   powershell -ExecutionPolicy Bypass -File install.ps1
param(
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
$svcName = "AdbTetherService"
$bin = Join-Path $PSScriptRoot "bin\Debug\net10.0-windows\win-x64\AdbTetherService.exe"

function IsAdmin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p = New-Object Security.Principal.WindowsPrincipal($id)
    $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (IsAdmin)) {
    Write-Error "Run this script as Administrator."
    exit 1
}

if (-not (Test-Path $bin)) {
    Write-Error "Binary not found: $bin"
    exit 1
}

if ($Uninstall) {
    $svc = Get-Service -Name $svcName -ErrorAction SilentlyContinue
    if ($svc) {
        # Best-effort: ask the running service (SYSTEM) to delete the persistent
        # wintun adapter before we stop/remove it. If the service is already
        # stopped, the adapter is removed by the dedicated uninstaller instead.
        try {
            $client = New-Object Net.Sockets.TcpClient
            $client.Connect("127.0.0.1", 47012)
            $stream = $client.GetStream()
            $payload = [Text.Encoding]::UTF8.GetBytes('{"op":"remove_adapter"}' + "`n")
            $stream.Write($payload, 0, $payload.Length)
            $stream.Flush()
            $client.Close()
            Write-Host "Adapter removal requested."
        } catch {
            Write-Host "WARNING: could not reach service to remove adapter (it will be removed by the uninstaller)."
        }
        Stop-Service -Name $svcName -Force -ErrorAction SilentlyContinue
        sc.exe delete $svcName | Out-Null
        Write-Host "Service removed."
    } else {
        Write-Host "Service not installed."
    }
    exit 0
}

$svc = Get-Service -Name $svcName -ErrorAction SilentlyContinue
if ($svc) {
    Write-Host "Service already exists (state=$($svc.Status)). Stopping and removing..."
    Stop-Service -Name $svcName -Force -ErrorAction SilentlyContinue
    sc.exe delete $svcName | Out-Null
    Start-Sleep -Seconds 2
}

Write-Host "Installing $svcName from $bin ..."
sc.exe create $svcName binPath= "`"$bin`" --service" start= auto DisplayName= "AdbTether Service" obj= "LocalSystem" | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Error "sc create failed"; exit 1 }

Write-Host "Starting $svcName ..."
Start-Service -Name $svcName
Start-Sleep -Seconds 2
$svc = Get-Service -Name $svcName
Write-Host "Service state: $($svc.Status)"
if ($svc.Status -ne "Running") {
    Write-Host "WARNING: service not running. Check Application event log."
    exit 1
}
Write-Host "OK."
