param(
    [Parameter(Mandatory=$true)][string]$Source,
    [Parameter(Mandatory=$true)][string]$VarName,
    [Parameter(Mandatory=$true)][string]$Group,
    [Parameter(Mandatory=$true)][string]$DirRef,
    [Parameter(Mandatory=$true)][string]$OutFile,
    [string]$Prefix = '',
    [switch]$Service
)

$ErrorActionPreference = 'Stop'
$srcRoot = (Resolve-Path $Source).Path.TrimEnd('\')
$script:n = 0
$script:vn = $VarName
$script:px = $Prefix
$script:isSvc = [bool]$Service

function New-Id([string]$p) { $script:n++; return "$($script:px)$p$($script:n)" }
# Deterministic component GUIDs: derived from the file's path inside its root,
# so regenerating a fragment after an unrelated build produces no diff.
function New-GuidAttr([string]$key) {
    $md5 = [System.Security.Cryptography.MD5]::Create()
    $b = $md5.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($key))
    $h = -join ($b | ForEach-Object { $_.ToString('x2') })
    return ('{' + $h.Substring(0, 8) + '-' + $h.Substring(8, 4) + '-' + $h.Substring(12, 4) + '-' + $h.Substring(16, 4) + '-' + $h.Substring(20, 12) + '}').ToUpper()
}

$sb  = New-Object System.Text.StringBuilder
$grp = New-Object System.Collections.Generic.List[string]

function Add-FileComponent([System.IO.FileInfo]$f, [int]$depth) {
    $id   = New-Id 'C'
    $fid  = New-Id 'F'
    $rel  = $f.FullName.Substring($script:srcRoot.Length + 1)
    $gid  = New-GuidAttr "$($script:vn)/$rel"
    $pad  = ' ' * ($depth + 3)
    [void]$script:sb.AppendLine("$pad<Component Id=`"$id`" Guid=`"$gid`">")
    [void]$script:sb.AppendLine(("$pad  " ) + "<File Id=`"$fid`" Name=`"$($f.Name)`" Source=`"`$($script:vn)\$rel`" KeyPath=`"yes`" />")
    if ($script:isSvc -and $f.Name -eq 'AdbTetherService.exe') {
        [void]$script:sb.AppendLine("$pad  <ServiceInstall Id=`"SvcInstall`" Name=`"AdbTetherService`" DisplayName=`"AdbTether Service`" Description=`"AdbTether background engine (wintun adapter + adb proxy bridge).`" Type=`"ownProcess`" Start=`"auto`" ErrorControl=`"normal`" Arguments=`"--service`" Vital=`"yes`" />")
        [void]$script:sb.AppendLine("$pad  <ServiceControl Id=`"SvcControl`" Name=`"AdbTetherService`" Start=`"install`" Stop=`"both`" Remove=`"uninstall`" Wait=`"yes`" />")
    }
    [void]$script:sb.AppendLine("$pad</Component>")
    $script:grp.Add($id)
}

function Walk([System.IO.DirectoryInfo]$d, [int]$depth) {
    foreach ($f in ($d.GetFiles() | Sort-Object Name)) {
        Add-FileComponent $f $depth
    }
    foreach ($sub in ($d.GetDirectories() | Sort-Object Name)) {
        $did = New-Id 'D'
        $pad = ' ' * ($depth + 2)
        [void]$script:sb.AppendLine("$pad<Directory Id=`"$did`" Name=`"$($sub.Name)`">")
        Walk $sub ($depth + 1)
        [void]$script:sb.AppendLine("$pad</Directory>")
    }
}

$rootId = if ($DirRef) { $DirRef } else { 'ROOT' }

[void]$sb.AppendLine('<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">')
[void]$sb.AppendLine('  <Fragment>')
[void]$sb.AppendLine("    <DirectoryRef Id=`"$DirRef`">")

# --- walk ---
$script:isSvc = [bool]$Service
foreach ($f in ((Get-ChildItem -LiteralPath $srcRoot -File) | Sort-Object Name)) { Add-FileComponent $f 0 }
foreach ($d in ((Get-ChildItem -LiteralPath $srcRoot -Directory) | Sort-Object Name)) {
    $did = New-Id 'D'
    [void]$sb.AppendLine("      <Directory Id=`"$did`" Name=`"$($d.Name)`">")
    Walk $d 1
    [void]$sb.AppendLine("      </Directory>")
}

[void]$sb.AppendLine('    </DirectoryRef>')
[void]$sb.AppendLine("    <ComponentGroup Id=`"$Group`">")
foreach ($c in $grp) { [void]$sb.AppendLine("      <ComponentRef Id=`"$c`" />") }
[void]$sb.AppendLine('    </ComponentGroup>')
[void]$sb.AppendLine('  </Fragment>')
[void]$sb.AppendLine('</Wix>')

[System.IO.File]::WriteAllText($OutFile, $sb.ToString())
Write-Host "yazildi: $OutFile ($($grp.Count) bilesen)"


