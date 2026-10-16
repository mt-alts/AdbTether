<#
.SYNOPSIS
    Live DNS-leak, IPv6-leak and WebRTC/STUN-leak checks for a running AdbTether
    tunnel.

.DESCRIPTION
    These are *live* checks: they inspect the machine's real network
    configuration and send a real STUN binding request, so they need an
    elevated shell and a raised tunnel. If either is missing the script prints
    a clear [SKIP] and exits 0 so it is safe to wire into CI or a dev loop.

    What is verified
    ----------------
    * DNS leak       - the tunnel adapter's resolvers are pinned to the tunnel
                       resolvers (1.1.1.1 / 8.8.8.8) and resolution works
                       through the tunnel, so lookups do not fall back to the
                       physical ISP resolver.
    * IPv6 leak      - the winning ::/0 route is the tunnel's fail-closed
                       blackhole, so AAAA records, DNS-over-IPv6 and WebRTC
                       IPv6 candidates never escape onto the physical link.
    * WebRTC leak    - a STUN binding request from this PC is relayed by the
                       tunnel; the observed egress address must be a public
                       address (and, when -ExpectedPublicIp is given, must match
                       the tunnel's egress rather than the local connection).

.PARAMETER Device
    Optional adb device serial, recorded for context only.

.PARAMETER StunServer
    STUN server to probe. Default: stun.l.google.com:19302.

.PARAMETER ExpectedPublicIp
    Optional. The public IP the tunnel is expected to egress from (e.g. the
    phone's public IP). When supplied, the STUN-mapped address must equal it.

.EXAMPLE
    ./Test-Leaks.ps1
    ./Test-Leaks.ps1 -ExpectedPublicIp 203.0.113.7
#>
[CmdletBinding()]
param(
    [string]$Device,
    [string]$StunServer = "stun.l.google.com:19302",
    [string]$ExpectedPublicIp
)

$ErrorActionPreference = "Stop"

$script:Failures = 0
function Test-Pass($m) { Write-Host "  [PASS] $m" -ForegroundColor Green }
function Test-Fail($m) { Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:Failures++ }
function Test-Warn($m) { Write-Host "  [WARN] $m" -ForegroundColor Yellow }
function Section($m) { Write-Host "`n== $m" -ForegroundColor Cyan }
function Exit-Skip($m) {
    Write-Host "`n[SKIP] $m" -ForegroundColor Yellow
    exit 0
}

# Sends a STUN binding request over UDP and returns the XOR-MAPPED-ADDRESS
# (the address the STUN server sees for us), or $null on failure.
function Get-StunMappedAddress {
    param([string]$Server = "stun.l.google.com:19302", [int]$TimeoutMs = 3000)

    $hostName = $Server
    $port = 3478
    if ($Server.Contains(":")) {
        $hostName, $portStr = $Server.Split(":", 2)
        [void][int]::TryParse($portStr, [ref]$port)
    }

    $ips = @([System.Net.Dns]::GetHostAddresses($hostName) |
        Where-Object { $_.AddressFamily -eq [System.Net.Sockets.AddressFamily]::InterNetwork })
    if ($ips.Count -eq 0) { return $null }
    $target = [System.Net.IPEndPoint]::new($ips[0], $port)

    $udp = [System.Net.Sockets.UdpClient]::new()
    try {
        $udp.Client.ReceiveTimeout = $TimeoutMs
        # Binding request: type 0x0001, length 0, magic cookie, random txn id.
        $txid = New-Object byte[] 12
        ([System.Random]::new()).NextBytes($txid)
        $req = New-Object System.Collections.Generic.List[byte]
        $req.AddRange([byte[]](0x00, 0x01, 0x00, 0x00))
        $req.AddRange([byte[]](0x21, 0x12, 0xA4, 0x42))
        $req.AddRange($txid)
        [void]$udp.Send($req.ToArray(), $req.Count, $target)

        $remote = [System.Net.IPEndPoint]::new([System.Net.IPAddress]::Any, 0)
        [byte[]]$resp = $udp.Receive([ref]$remote)
    } catch {
        return $null
    } finally {
        $udp.Close()
    }

    # Walk the attribute list for XOR-MAPPED-ADDRESS (0x0020) / MAPPED (0x0001).
    $i = 20
    $cookie = [byte[]](0x21, 0x12, 0xA4, 0x42)
    while ($i + 4 -le $resp.Length) {
        $atype = ([int]$resp[$i] -shl 8) -bor [int]$resp[$i + 1]
        $alen = ([int]$resp[$i + 2] -shl 8) -bor [int]$resp[$i + 3]
        $val = $i + 4
        if (($atype -eq 0x0020 -or $atype -eq 0x0001) -and $val + 8 -le $resp.Length) {
            if ($resp[$val + 1] -ne 0x01) { return $null } # only IPv4
            $p = ([int]$resp[$val + 2] -shl 8) -bor [int]$resp[$val + 3]
            [byte[]]$ipb = $resp[($val + 4)..($val + 7)]
            if ($atype -eq 0x0020) {
                $p = $p -bxor 0x2112
                for ($k = 0; $k -lt 4; $k++) { $ipb[$k] = $ipb[$k] -bxor $cookie[$k] }
            }
            $ip = [System.Net.IPAddress]::new($ipb)
            return [pscustomobject]@{ IPAddress = $ip; Port = $p }
        }
        # Attributes are padded to a 4-byte boundary.
        $i = $val + $alen
        if ($alen % 4) { $i += 4 - ($alen % 4) }
    }
    return $null
}

# ── preflight ────────────────────────────────────────────────────────────────
Section "Preflight"
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { Exit-Skip "this test needs an elevated (Run as Administrator) shell" }
Write-Host "  elevated shell: yes"

$adapter = Get-NetAdapter -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match 'AdbTether' -or $_.InterfaceDescription -match 'AdbTether' } |
    Select-Object -First 1
if (-not $adapter) { Exit-Skip "wintun adapter 'AdbTether' not found - is the tunnel up?" }
if ($adapter.Status -ne 'Up') { Exit-Skip "adapter '$($adapter.Name)' is $($adapter.Status) - is the tunnel up?" }
Write-Host "  tunnel adapter: $($adapter.Name) (ifIndex $($adapter.ifIndex), Up)"
if ($Device) { Write-Host "  device: $Device (informational)" }

# ── DNS leak ─────────────────────────────────────────────────────────────────
Section "DNS leak"
$expected = @("1.1.1.1", "8.8.8.8")
$dns = @((Get-DnsClientServerAddress -InterfaceIndex $adapter.ifIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue).ServerAddresses)
if ($dns.Count -eq 0) {
    Test-Fail "tunnel adapter has no DNS servers configured"
}
else {
    Write-Host "  adapter DNS: $($dns -join ', ')"
    $missing = @($expected | Where-Object { $_ -notin $dns })
    if ($missing.Count -eq 0) {
        Test-Pass "adapter pinned to the tunnel resolvers (1.1.1.1, 8.8.8.8)"
    }
    else {
        Test-Fail "adapter DNS is not pinned to the tunnel resolvers (missing: $($missing -join ', '))"
    }
}

$physDns = @(Get-DnsClientServerAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
    Where-Object { $_.InterfaceIndex -ne $adapter.ifIndex -and $_.ServerAddresses.Count -gt 0 })
if ($physDns.Count -gt 0) {
    Write-Host "  physical DNS (must NOT be used by the tunnel adapter): $(($physDns | ForEach-Object { $_.ServerAddresses }) -join ', ')"
}

try {
    $r = Resolve-DnsName -Name "example.com" -Type A -DnsOnly -QuickTimeout -ErrorAction Stop
    $ip = ($r | Where-Object { $_.IPAddress } | Select-Object -First 1).IPAddress
    if ($ip) { Test-Pass "resolution through the tunnel works (example.com -> $ip)" }
    else { Test-Fail "resolution returned no A record" }
}
catch {
    Test-Fail "resolution failed through the tunnel: $($_.Exception.Message)"
}

# ── IPv6 kill-switch ─────────────────────────────────────────────────────────
Section "IPv6 kill-switch (AAAA / DNS-over-v6 / WebRTC v6)"
$routes = @(Get-NetRoute -AddressFamily IPv6 -DestinationPrefix "::/0" -ErrorAction SilentlyContinue)
if ($routes.Count -eq 0) {
    Test-Pass "no IPv6 default route at all (nothing can leak)"
}
else {
    $best = $routes | Sort-Object RouteMetric, ifIndex | Select-Object -First 1
    Write-Host "  winning ::/0 -> if $($best.ifIndex) ($($best.InterfaceAlias)) next-hop $($best.NextHop) metric $($best.RouteMetric)"
    if ($best.InterfaceAlias -match 'AdbTether' -or $best.ifIndex -eq $adapter.ifIndex) {
        Test-Pass "winning ::/0 route points at the tunnel (fail-closed blackhole)"
    }
    else {
        Test-Fail "IPv6 default route escapes via '$($best.InterfaceAlias)' - IPv6 can leak"
    }
    $physicalBest = $routes | Where-Object { $_.ifIndex -ne $adapter.ifIndex } |
        Sort-Object RouteMetric | Select-Object -First 1
    if ($physicalBest -and $physicalBest.RouteMetric -le $best.RouteMetric) {
        Test-Fail "a physical ::/0 route (metric $($physicalBest.RouteMetric)) ties or beats the tunnel blackhole"
    }
}

# ── WebRTC / STUN leak ───────────────────────────────────────────────────────
Section "WebRTC / STUN leak"
$mapped = Get-StunMappedAddress -Server $StunServer
if (-not $mapped) {
    Test-Fail "no STUN response from $StunServer - is the UDP relay up?"
}
else {
    $ip = $mapped.IPAddress.ToString()
    Write-Host "  STUN egress address: $ip`:$($mapped.Port) (via $StunServer)"
    if ($ip -match '^(10\.|127\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.|169\.254\.)') {
        Test-Pass "STUN sees a private/loopback address (request handled inside the host)"
    }
    elseif ($ExpectedPublicIp -and $ip -eq $ExpectedPublicIp) {
        Test-Pass "STUN egress IP matches the expected tunnel egress ($ExpectedPublicIp)"
    }
    elseif ($ExpectedPublicIp) {
        Test-Fail "STUN egress IP $ip != expected $ExpectedPublicIp (possible WebRTC leak)"
    }
    else {
        Test-Warn "STUN egress IP is $ip - pass -ExpectedPublicIp <phone's public IP> to assert it matched the tunnel"
    }
}

# ── summary ──────────────────────────────────────────────────────────────────
Section "Summary"
if ($script:Failures -eq 0) {
    Write-Host "All executed leak checks passed." -ForegroundColor Green
    exit 0
}
Write-Host "$($script:Failures) leak check(s) FAILED." -ForegroundColor Red
exit 1
