<#
.SYNOPSIS
    Runs the AdbTether test suite.

.DESCRIPTION
    Default: the fast, self-contained Rust unit + integration tests (no device,
    no admin, no tunnel required) via `cargo test --workspace`.

    -Leaks additionally runs the live DNS / IPv6 / WebRTC leak checks in
    tests\leak\Test-Leaks.ps1, which need an elevated shell and a raised tunnel
    (they SKIP gracefully when those are missing).

.PARAMETER Leaks
    Also run the live leak checks.

.PARAMETER Device
    Optional adb device serial for the live leak checks.

.PARAMETER ExpectedPublicIp
    Optional expected tunnel egress IP for the live WebRTC/STUN leak check.

.EXAMPLE
    ./test.ps1
    ./test.ps1 -Leaks -ExpectedPublicIp 203.0.113.7
#>
[CmdletBinding()]
param(
    [switch]$Leaks,
    [string]$Device,
    [string]$ExpectedPublicIp
)

$ErrorActionPreference = "Stop"
# Native tools write progress to stderr; make sure that never becomes a
# terminating error. Exit codes are checked explicitly.
if (Get-Variable PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}
$Root = $PSScriptRoot
Push-Location $Root

function Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }

try {
    Step "Rust unit + integration tests (engine + ws-proxy)"
    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { Write-Host "[!] Rust tests failed" -ForegroundColor Red; exit 1 }

    if ($Leaks) {
        Step "Live DNS / IPv6 / WebRTC leak checks"
        $args = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "tests\leak\Test-Leaks.ps1")
        if ($Device) { $args += @("-Device", $Device) }
        if ($ExpectedPublicIp) { $args += @("-ExpectedPublicIp", $ExpectedPublicIp) }
        powershell @args
        if ($LASTEXITCODE -ne 0) { Write-Host "[!] Leak checks failed" -ForegroundColor Red; exit 1 }
    }
    else {
        Write-Host "`n(Skipping live leak checks; pass -Leaks to run them.)" -ForegroundColor Yellow
    }

    Write-Host "`nAll requested tests passed." -ForegroundColor Green
}
finally {
    Pop-Location
}
