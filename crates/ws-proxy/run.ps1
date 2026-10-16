param(
    [int]$Port = 1092,
    [string]$BindAddr = "127.0.0.1",
    [string]$RemotePath = "/data/local/tmp/ws-proxy",
    [switch]$BuildOnly,
    [switch]$RunOnly,
    [switch]$Forward,
    [string]$Device = "",
    [switch]$Verbose,
    [switch]$LogOnly,
    [switch]$Daemon,
    [switch]$Stop
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProxyDir = $ScriptDir
# Cargo workspace root (this crate lives at crates/ws-proxy); the shared
# target dir that cargo actually writes to lives at the repo root.
$WorkspaceRoot = (Resolve-Path (Join-Path $ScriptDir '..\..')).Path
$TargetDir = Join-Path $WorkspaceRoot 'target'
$TargetTriple = "aarch64-linux-android"
$BinaryName = "ws-proxy"
$BinaryPath = "$TargetDir\$TargetTriple\release\$BinaryName.exe"
$DeviceBinary = "$TargetDir\$TargetTriple\release\$BinaryName"
$RemotePath = "/data/local/tmp/ws-proxy"
$DeviceArg = if ($Device) { "-s $Device" } else { "" }

function Write-Step { param($msg) Write-Host "`n[+] $msg" -ForegroundColor Green }
function Write-Info { param($msg) Write-Host "    $msg" }

# --- Logcat only mode ---
if ($LogOnly) {
    Write-Host "[+] Watching WebSocket proxy logs (Ctrl+C to stop)..." -ForegroundColor Cyan
    & cmd /c "adb $DeviceArg logcat -v brief -s ws-proxy:D *:S 2>&1"
    exit 0
}

# --- Stop running proxy ---
if ($Stop) {
    Write-Step "Stopping WebSocket proxy on device..."
    & cmd /c "adb $DeviceArg shell pkill -f ws-proxy 2>&1"
    if ($LASTEXITCODE -eq 0) {
        Write-Info "Proxy process killed."
    } else {
        Write-Info "No running proxy found (already stopped?)."
    }
    & cmd /c "adb $DeviceArg forward --remove tcp:$Port 2>&1" | Out-Null
    Write-Info "Removed port forward tcp:$Port"
    exit 0
}

# --- Build ---
if (-not $RunOnly) {
    Write-Step "Building WebSocket Proxy (Rust + tokio-tungstenite)..."

    # Auto-install Rust target if missing
    $installed = rustup target list --installed 2>&1
    if ($installed -notcontains $TargetTriple) {
        Write-Info "Installing Rust target: $TargetTriple ..."
        rustup target add $TargetTriple
        if ($LASTEXITCODE -ne 0) {
            Write-Host "[!] Failed to install Rust target" -ForegroundColor Red
            exit 1
        }
    }

    # Find Android NDK
    $SdkDir = "$env:LOCALAPPDATA\Android\Sdk"
    $NdkDir = Get-ChildItem "$SdkDir\ndk" -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending | Select-Object -First 1
    if (-not $NdkDir) {
        Write-Host "[!] Android NDK not found at $SdkDir\ndk" -ForegroundColor Red
        Write-Host "    Install via Android Studio > SDK Manager > NDK" -ForegroundColor Red
        exit 1
    }
    Write-Info "NDK: $($NdkDir.Name)"

    # Find clang linker
    $ClangDir = "$($NdkDir.FullName)\toolchains\llvm\prebuilt\windows-x86_64\bin"
    $Linker = Get-ChildItem "$ClangDir\aarch64-linux-android*-clang.cmd" -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending | Select-Object -First 1
    if (-not $Linker) {
        Write-Host "[!] aarch64 clang not found in NDK" -ForegroundColor Red
        exit 1
    }
    Write-Info "Linker: $($Linker.Name)"
    # Build — use rustup toolchain (system cargo may lack Android targets)
    $env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $Linker.FullName
    $env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-gnu"
    # Prepend rustup toolchain bin so cargo/rustc resolve correctly
    $RustupBin = "$env:USERPROFILE\.rustup\toolchains\$env:RUSTUP_TOOLCHAIN\bin"
    $env:PATH = "$RustupBin;$env:PATH"

    Push-Location $ProxyDir
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    cargo build --target $TargetTriple --release 2>&1 | ForEach-Object { Write-Host $_ }
    $buildOk = $LASTEXITCODE
    $ErrorActionPreference = $prevEap
    Pop-Location
    if ($buildOk -ne 0) {
        Write-Host "[!] Build failed" -ForegroundColor Red
        exit 1
    }

    # Rename to strip .exe for Linux target
    $builtPath = "$TargetDir\$TargetTriple\release\$BinaryName"
    if (-not (Test-Path $builtPath)) {
        # On Windows, cargo may produce .exe even for cross targets
        $builtPath = "$TargetDir\$TargetTriple\release\$BinaryName.exe"
    }
    if (-not (Test-Path $builtPath)) {
        Write-Host "[!] Binary not found after build" -ForegroundColor Red
        exit 1
    }
    $BinaryPath = $builtPath
    Write-Info "Built: $BinaryPath ($(([math]::Round((Get-Item $BinaryPath).Length / 1MB, 1))) MB)"
}

if ($BuildOnly) { exit 0 }

# --- Kill existing proxy ---
Write-Step "Stopping any existing proxy on device..."
& cmd /c "adb $DeviceArg shell pkill -f ws-proxy 2>&1" | Out-Null
Start-Sleep -Seconds 1
& cmd /c "adb $DeviceArg forward --remove tcp:$Port 2>&1" | Out-Null
Write-Info "Cleared old process and port forward"

# --- Push to device ---
Write-Step "Pushing to device..."
& cmd /c "adb $DeviceArg push $BinaryPath $RemotePath 2>&1"
if ($LASTEXITCODE -ne 0) {
    Write-Host "[!] adb push failed. Is device connected?" -ForegroundColor Red
    exit 1
}
& cmd /c "adb $DeviceArg shell chmod 755 $RemotePath 2>&1" | Out-Null

# --- Port forward ---
if ($Forward) {
    & cmd /c "adb $DeviceArg forward tcp:$Port tcp:$Port 2>&1"
    Write-Info "Port forwarding: localhost:$Port -> device:$Port"
}

# --- Extra options ---
$ExtraArgs = ""
if ($Verbose) { $ExtraArgs = "-v" }

Write-Step "Starting WebSocket proxy on port $Port ..."
Write-Host ""
Write-Host "  Proxy: 127.0.0.1:$Port" -ForegroundColor Cyan
Write-Host "  Mode:  WebSocket proxy (CONNECT + UDP relay)" -ForegroundColor Cyan
    Write-Host "  Logs:  adb logcat -s ws-proxy" -ForegroundColor DarkGray
    Write-Host ""

    $RustLog = if ($Verbose) { "RUST_LOG=debug" } else { "RUST_LOG=info" }

    if ($Daemon) {
        & cmd /c "adb $DeviceArg shell `"nohup sh -c '$RustLog $RemotePath -p $Port -b $BindAddr $ExtraArgs' </dev/null >/data/local/tmp/ws-proxy.log 2>&1 &`" 2>&1"
    Write-Host "  Proxy started in background (PID on device)." -ForegroundColor Green
    Write-Host "  Use: adb logcat -s ws-proxy:D" -ForegroundColor DarkGray
} else {
    Write-Host "  Press Ctrl+C to stop" -ForegroundColor Yellow
    Write-Host ""
        & cmd /c "adb $DeviceArg shell `"sh -c '$RustLog $RemotePath -p $Port -b $BindAddr $ExtraArgs'`" 2>&1"
}
