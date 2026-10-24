# Mac motorunun (crates/proxy) yontemlerini bu hatta canli dener: her yontem icin dpi-proxy'yi
# baslatir, engelli bilinen siteleri curl.exe ile proxy uzerinden ister, sonucu tablo yapar.
# "none" yontemi parcalama yapmaz (sifreli DNS acik): DPI engelini gosteren karsilastirma.
#
#   powershell -File perf\proxy-live-test.ps1              # DPI UI aciksa uyarir ve durur
#   powershell -File perf\proxy-live-test.ps1 -PauseDpiUi  # DPI UI'yi test boyunca kapatir, sonra geri acar
#
# DPI UI acikken Windows motoru proxy'nin trafigini de duzeltir; sonuc anlamsiz olur.
# Not: dosya bilerek yalnizca ASCII karakter icerir (PowerShell 5.1 BOM'suz dosyayi ANSI okur).
param(
    [switch] $PauseDpiUi,
    [switch] $AllowDpiUi,
    [string[]] $Methods = @('none', 'disorder', 'tlsrec', 'tlsrecdisorder', 'disordersni', 'splitsni', 'split'),
    [string[]] $Targets = @(
        'https://discord.com/',
        'https://gateway.discord.gg/',
        'https://cdn.discordapp.com/',
        'https://www.roblox.com/',
        'https://www.wattpad.com/',
        'https://example.com/'
    ),
    [int] $MaxTime = 7
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $PSScriptRoot 'out\proxy-live'
New-Item -ItemType Directory -Force $out | Out-Null

function Send-Pipe([string[]] $requests) {
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'GoodbyeDPI3', [System.IO.Pipes.PipeDirection]::InOut)
    try { $pipe.Connect(2000) } catch { return @() }
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $w = New-Object System.IO.StreamWriter($pipe, $utf8); $w.AutoFlush = $true
    $r = New-Object System.IO.StreamReader($pipe, $utf8)
    $lines = @(); $id = 1
    foreach ($q in @('{"type":"hello","client":"proxy-live-test","protocol":1}') + $requests) {
        $w.WriteLine('{"id":' + $id + ',"request":' + $q + '}')
        while ($true) {
            $line = $r.ReadLine()
            if ($null -eq $line) { break }
            $lines += $line
            if ($line -match '"kind":"reply"') { break }
        }
        $id++
    }
    $pipe.Dispose()
    return $lines
}

function Get-EngineState {
    $lines = Send-Pipe @('{"type":"getState"}')
    foreach ($l in $lines) { if ($l -match '"engine":\{"state":"(\w+)"') { return $Matches[1] } }
    return 'none'
}

function Wait-EngineState([string] $want, [int] $seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        if ((Get-EngineState) -eq $want) { return $true }
        Start-Sleep -Milliseconds 300
    }
    return $false
}

function Wait-Port([int] $port) {
    for ($i = 0; $i -lt 50; $i++) {
        $c = New-Object System.Net.Sockets.TcpClient
        try { $c.Connect('127.0.0.1', $port); $c.Close(); return $true } catch { Start-Sleep -Milliseconds 100 } finally { $c.Dispose() }
    }
    return $false
}

Write-Host 'dpi-proxy derleniyor...'
Push-Location $root
& cargo build --release -q -p proxy --bin dpi-proxy 2>&1 | Out-Host
Pop-Location
$exe = Join-Path $root 'target\release\dpi-proxy.exe'
if (-not (Test-Path $exe)) { throw 'dpi-proxy.exe yok' }

$engine = Get-EngineState
$paused = $false
if ($engine -eq 'on' -or $engine -eq 'paused' -or $engine -eq 'starting') {
    if ($PauseDpiUi) {
        Write-Host 'DPI UI test boyunca kapatiliyor (sonunda geri acilir)...'
        Send-Pipe @('{"type":"disconnect"}') | Out-Null
        if (-not (Wait-EngineState 'off' 15)) { throw 'DPI UI kapanmadi' }
        $paused = $true
        Start-Sleep -Milliseconds 500
    } elseif (-not $AllowDpiUi) {
        Write-Host 'DPI UI acik: Windows motoru sonucu degistirir. -PauseDpiUi ile calistir (ya da DPI UI''de Kes).'
        exit 3
    } else {
        Write-Host 'UYARI: DPI UI acik, sonuclar yalnizca araclarin calistigini gosterir.'
    }
}

$results = @{}
$times = @{}
$port = 47820
try {
    foreach ($m in $Methods) {
        $port++
        $log = Join-Path $out "$m.log"
        $p = Start-Process -FilePath $exe -ArgumentList @('--port', $port, '--method', $m, '--verbose') -NoNewWindow -PassThru -RedirectStandardError $log -RedirectStandardOutput (Join-Path $out "$m.out")
        try {
            if (-not (Wait-Port $port)) { throw "dpi-proxy $m acilmadi" }
            $jobs = @()
            $i = 0
            foreach ($t in $Targets) {
                $i++
                $f = Join-Path $out "$m-$i.txt"
                $cargs = @('-s', '-o', 'NUL', '-x', "http://127.0.0.1:$port", '--connect-timeout', '5', '--max-time', $MaxTime, '-w', '%{http_code}:%{time_total}', $t)
                $jobs += [pscustomobject]@{ Target = $t; File = $f; Proc = (Start-Process -FilePath 'curl.exe' -ArgumentList $cargs -NoNewWindow -PassThru -RedirectStandardOutput $f) }
            }
            foreach ($j in $jobs) {
                $j.Proc.WaitForExit(($MaxTime + 5) * 1000) | Out-Null
                $text = (Get-Content $j.File -Raw -ErrorAction SilentlyContinue)
                if (-not $text) { $text = '000:0' }
                $parts = $text.Trim().Split(':')
                $results["$m|$($j.Target)"] = $parts[0]
                $times["$m|$($j.Target)"] = [double]::Parse($parts[1].Replace(',', '.'), [Globalization.CultureInfo]::InvariantCulture)
            }
        } finally {
            Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
        }
    }
} finally {
    if ($paused) {
        Write-Host 'DPI UI geri aciliyor...'
        Send-Pipe @('{"type":"connect"}') | Out-Null
        if (Wait-EngineState 'on' 20) { Write-Host 'DPI UI yeniden acik.' } else { Write-Host 'UYARI: DPI UI kendiliginden acilmadi, uygulamadan Baglan''a bas.' }
    }
}

# Tablo: satir site, sutun yontem. Hucre: HTTP kodu ve sure; 000 = baglanti kurulamadi.
$w = 22
$header = '{0,-30}' -f 'site'
foreach ($m in $Methods) { $header += ('{0,-' + $w + '}') -f $m }
Write-Host ''
Write-Host $header
foreach ($t in $Targets) {
    $row = '{0,-30}' -f ($t -replace '^https://', '')
    foreach ($m in $Methods) {
        $code = $results["$m|$t"]
        $cell = if ($code -eq '000') { 'YOK' } else { '{0} {1:N2}s' -f $code, $times["$m|$t"] }
        $row += ('{0,-' + $w + '}') -f $cell
    }
    Write-Host $row
}
Write-Host ''
foreach ($m in $Methods) {
    $ok = @($Targets | Where-Object { $results["$m|$_"] -ne '000' }).Count
    Write-Host ('{0,-16} {1}/{2} site acildi' -f $m, $ok, $Targets.Count)
}
Write-Host "Ayrintili kayitlar: $out"
