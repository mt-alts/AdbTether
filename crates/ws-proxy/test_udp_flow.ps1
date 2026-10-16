$ErrorActionPreference = 'Stop'

function ToBE([uint16]$v) {
    $b = [BitConverter]::GetBytes($v)
    [Array]::Reverse($b)
    return ,$b
}

function Add-Range($list, [byte[]]$arr) { foreach ($x in $arr) { $list.Add($x) } }

function Build-DnsQuery([uint16]$tid) {
    $l = New-Object System.Collections.Generic.List[byte]
    Add-Range $l (ToBE $tid)
    Add-Range $l ([byte[]]@(0x01,0x00, 0x00,0x01, 0x00,0x00, 0x00,0x00, 0x00,0x00))
    Add-Range $l ([Text.Encoding]::ASCII.GetBytes('example'))
    $l.Add(3)
    Add-Range $l ([Text.Encoding]::ASCII.GetBytes('com'))
    $l.Add(0)
    Add-Range $l ([byte[]]@(0x00,0x01, 0x00,0x01))
    return ,$l.ToArray()
}

function New-Frame([uint16]$fid, [byte[]]$ip, [uint16]$port, [byte[]]$payload) {
    $f = New-Object System.Collections.Generic.List[byte]
    Add-Range $f (ToBE $fid)
    $f.Add(0); $f.Add(1)
    Add-Range $f $ip
    Add-Range $f (ToBE $port)
    Add-Range $f $payload
    return ,$f.ToArray()
}

# --- yerel UDP echo: 127.0.0.1:17000 (deterministik, LAN gurultusuz) ---
$echo = New-Object System.Net.Sockets.UdpClient(17000)
$echo.Client.ReceiveTimeout = 0
function Pump-Echo {
    while ($echo.Available -gt 0) {
        $any = [Net.IPEndPoint]::new([Net.IPAddress]::Any, 0)
        $d = $echo.Receive([ref]$any)
        $null = $echo.Send($d, $d.Length, $any)
    }
}

$ws = New-Object System.Net.WebSockets.ClientWebSocket
$ct = [Threading.CancellationToken]::None
$ws.ConnectAsync([Uri]"ws://127.0.0.1:10809/", $ct).Wait()
Write-Host "[+] WS connected"

$hs = [Text.Encoding]::UTF8.GetBytes('{"op":"udp"}')
$ws.SendAsync([ArraySegment[byte]]::new($hs), [Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).Wait()

$ip = [byte[]]@(127,0,0,1)
$f1 = New-Frame 100 $ip 17000 (Build-DnsQuery 0x1111)
$f2 = New-Frame 200 $ip 17000 (Build-DnsQuery 0x2222)
$ws.SendAsync([ArraySegment[byte]]::new($f1), [Net.WebSockets.WebSocketMessageType]::Binary, $true, $ct).Wait()
Start-Sleep -Milliseconds 100; Pump-Echo
$ws.SendAsync([ArraySegment[byte]]::new($f2), [Net.WebSockets.WebSocketMessageType]::Binary, $true, $ct).Wait()
Write-Host "[+] sent fid=100/tid=0x1111 and fid=200/tid=0x2222 -> 127.0.0.1:17000"

$results = @{}
$expected = @{ 0x1111 = 100; 0x2222 = 200 }
for ($i = 0; $i -lt 8 -and $results.Count -lt 2; $i++) {
    $buf = New-Object byte[] 65536
    $acc = New-Object System.Collections.Generic.List[byte]
    $done = $false
    while (-not $done) {
        # WS beklemesini dilimlere bol, arada echo'yu servis et
        $r = $null
        for ($slice = 0; $slice -lt 25 -and -not $r; $slice++) {
            Pump-Echo
            $t = $ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $ct)
            if ($t.Wait(200)) { $r = $t.Result } else { $r = $null }
        }
        if (-not $r) { Write-Host "[-] TIMEOUT"; break }
        for ($k = 0; $k -lt $r.Count; $k++) { $acc.Add($buf[$k]) }
        if ($r.EndOfMessage) { $done = $true }
    }
    $n = $acc.Count
    if ($n -lt 14) { Write-Host "[-] junk/short frame ($n) atlandi"; continue }
    $fid   = [uint16]$acc[0] * 256 + [uint16]$acc[1]
    $tid   = [uint16]$acc[10] * 256 + [uint16]$acc[11]
    if (-not $expected.ContainsKey($tid)) {
        Write-Host ("[-] yabanci frame atlandi: fid={0} tid=0x{1:X4}" -f $fid, $tid)
        continue
    }
    $results[$fid] = @{ tid = $tid; len = $n }
    Write-Host ("[+] reply: fid={0} tid=0x{1:X4} bytes={2}" -f $fid, $tid, $n)
}

if ($results.ContainsKey(100) -and $results.ContainsKey(200) -and
    $results[100].tid -eq 0x1111 -and $results[200].tid -eq 0x2222) {
    Write-Host "`n=== PASS: her akis kendi yanitini dogru flow-id ile aldi ===" -ForegroundColor Green
} else {
    Write-Host "`n=== FAIL: demux bozuk veya yanit eksik ===" -ForegroundColor Red
}

$echo.Close()
$ws.Dispose()
