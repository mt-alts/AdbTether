<#
.SYNOPSIS
    Builds AdbTether end to end: Rust engine, phone-side ws-proxy, Windows
    service, WinUI 3 app, and (optionally) the MSI.

.DESCRIPTION
    Picks a single Rust toolchain for everything so host and Android artifacts
    never mix toolchains in the shared target dir. Prefers a rustup toolchain
    that already has the aarch64-linux-android target; otherwise adds it to the
    active toolchain.

    Android cross-build needs the Android NDK (auto-located under
    %LOCALAPPDATA%\Android\Sdk\ndk).

.PARAMETER Configuration
    .NET build configuration. Default: Release.

.PARAMETER SkipAndroid
    Do not build the phone-side ws-proxy (skips NDK).

.PARAMETER SkipMsi
    Do not rebuild the MSI.

.PARAMETER AllowMissingProxy
    Build the MSI even when the phone-side ws-proxy is absent from the payload.
    Only for throwaway/dev MSIs: a release must never ship without it.

.EXAMPLE
    ./build.ps1
    ./build.ps1 -Configuration Debug -SkipMsi
#>
[CmdletBinding()]
param(
    [string]$Configuration = "Release",
    [switch]$SkipAndroid,
    [switch]$SkipMsi,
    [switch]$AllowMissingProxy
)

$ErrorActionPreference = "Stop"
# Native tools (cargo, dotnet, wix) write progress to stderr; make sure that
# never turns into a terminating error. Exit codes are checked explicitly.
if (Get-Variable PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}
$Root = $PSScriptRoot
Push-Location $Root

function Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }
function Fail($msg) { Write-Host "[!] $msg" -ForegroundColor Red; Pop-Location; exit 1 }

try {
    # ------------------------------------------------------------------
    # 1. Pick one Rust toolchain (host + Android from the same toolchain)
    # ------------------------------------------------------------------
    $AndroidTarget = "aarch64-linux-android"
    $cargo = "cargo"
    $rustupAvailable = $null -ne (Get-Command rustup -ErrorAction SilentlyContinue)

    if ($rustupAvailable -and -not $SkipAndroid) {
        $candidate = $null
        foreach ($tc in (rustup toolchain list 2>$null | ForEach-Object { ($_ -split '\s+')[0] })) {
            if (-not $tc) { continue }
            $targets = rustup target list --installed --toolchain $tc 2>$null
            if ($targets -contains $AndroidTarget) { $candidate = $tc; break }
        }
        if (-not $candidate) {
            $candidate = (rustup show active-toolchain 2>$null).Split(' ')[0]
            if (-not $candidate) { $candidate = "stable" }
            Write-Host "Adding $AndroidTarget to toolchain '$candidate'..."
            rustup target add --toolchain $candidate $AndroidTarget | Out-Null
        }
        $tcBin = "$env:USERPROFILE\.rustup\toolchains\$candidate\bin"
        if (Test-Path "$tcBin\cargo.exe") {
            $env:PATH = "$tcBin;$env:PATH"
            $env:RUSTC = "$tcBin\rustc.exe"
            $cargo = "$tcBin\cargo.exe"
            Write-Host "Rust toolchain: $candidate"
        }
    }

    # ------------------------------------------------------------------
    # 2. Locate the Android NDK linker (for ws-proxy)
    # ------------------------------------------------------------------
    if (-not $SkipAndroid) {
        $ndkRoot = Join-Path $env:LOCALAPPDATA "Android\Sdk\ndk"
        $ndk = Get-ChildItem $ndkRoot -Directory -ErrorAction SilentlyContinue |
            Sort-Object Name -Descending | Select-Object -First 1
        if (-not $ndk) { Fail "Android NDK not found under $ndkRoot" }
        $ndkBin = Join-Path $ndk.FullName "toolchains\llvm\prebuilt\windows-x86_64\bin"
        $linker = Get-ChildItem (Join-Path $ndkBin "aarch64-linux-android*-clang.cmd") |
            Sort-Object Name -Descending | Select-Object -First 1
        if (-not $linker) { Fail "aarch64 clang not found in $ndkBin" }
        $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $linker.FullName
        Write-Host "NDK: $($ndk.Name)  linker: $($linker.Name)"
    }

    # ------------------------------------------------------------------
    # 3. Rust: engine (Windows cdylib) + ws-proxy (Android)
    # ------------------------------------------------------------------
    Step "Building Rust engine (AdbTether_core.dll)"
    & $cargo build --release -p adbtether-engine
    if ($LASTEXITCODE -ne 0) { Fail "engine build failed" }

    if (-not $SkipAndroid) {
        Step "Building ws-proxy (aarch64-linux-android)"
        & $cargo build --release -p adbtether-server --target $AndroidTarget
        if ($LASTEXITCODE -ne 0) { Fail "ws-proxy build failed" }
    }

    # ------------------------------------------------------------------
    # 4. .NET: service + WinUI 3 app
    # ------------------------------------------------------------------
    # Output dirs are wiped first so a rename or a deleted file can never
    # survive from an earlier build into the next one (and thus into the MSI).
    $uiBin  = "src\AdbTether\bin\x64\$Configuration\net10.0-windows10.0.19041.0"
    $svcBin = "src\AdbTetherService\bin\$Configuration\net10.0-windows\win-x64"
    foreach ($d in @($uiBin, $svcBin)) {
        if (Test-Path $d) {
            Write-Host "Cleaning $d"
            Remove-Item $d -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path $d) { Write-Host "  (files are locked; left in place)" -ForegroundColor Yellow }
        }
    }

    Step "Building Windows service"
    dotnet build "src\AdbTetherService\AdbTetherService.csproj" -c $Configuration
    if ($LASTEXITCODE -ne 0) { Fail "service build failed" }

    Step "Building WinUI 3 app (x64)"
    dotnet build "src\AdbTether\AdbTether.csproj" -c $Configuration -p:Platform=x64
    if ($LASTEXITCODE -ne 0) { Fail "UI build failed" }

    # ------------------------------------------------------------------
    # 5. MSI (Release only; needs the full payload)
    # ------------------------------------------------------------------
    if (-not $SkipMsi) {
        if ($Configuration -ne "Release") {
            Write-Host "Skipping MSI (only built for Release)." -ForegroundColor Yellow
        } else {
            # The phone-side proxy is copied into the UI payload only when it
            # has actually been cross-compiled, so an MSI built on a machine
            # without it would silently ship a tunnel that cannot work on
            # device. Refuse to build one unless explicitly allowed.
            if (-not (Test-Path (Join-Path $uiBin "ws-proxy"))) {
                if ($AllowMissingProxy) {
                    Write-Host "[!] ws-proxy missing from '$uiBin'; building anyway (-AllowMissingProxy)." -ForegroundColor Yellow
                } else {
                    Fail "ws-proxy is missing from '$uiBin', so the MSI would ship without the phone-side proxy. Build it (drop -SkipAndroid) or pass -AllowMissingProxy for a throwaway MSI."
                }
            }

            Step "Regenerating MSI file fragments"
            powershell -NoProfile -File "installer\gen_fragments.ps1" -Source $uiBin  -VarName UI  -Group AppFilesGroup    -DirRef APPDIR     -OutFile "installer\AppFiles.wxs"    -Prefix AF_
            powershell -NoProfile -File "installer\gen_fragments.ps1" -Source $svcBin -VarName SVC -Group ServiceFilesGroup -DirRef SERVICEDIR -OutFile "installer\ServiceFiles.wxs" -Prefix SV_ -Service

            Step "Building MSI"
            wix build -arch x64 -d "UI=$uiBin" -d "SVC=$svcBin" -o "installer\dist\AdbTether.msi" `
                "installer\Product.wxs" "installer\AppFiles.wxs" "installer\ServiceFiles.wxs"
            if ($LASTEXITCODE -ne 0) { Fail "MSI build failed" }
        }
    }

    Write-Host "`nBuild complete." -ForegroundColor Green
}
finally {
    Pop-Location
}
