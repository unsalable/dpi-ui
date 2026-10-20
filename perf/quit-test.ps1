# Pencere/tepsi yasam dongusu testi: gercek surecler, kullanicinin acik uygulamasindan ayri adlarla
# (GDPI3_INSTANCE). Ayarlara dokunmaz (GDPI3_TEST).
# Ornek: powershell -ExecutionPolicy Bypass -File perf\quit-test.ps1 -Exe target\release\GoodbyeDPI3.exe
# Not: dosya bilerek yalnizca ASCII karakter icerir (Windows PowerShell 5.1 BOM'suz UTF-8'i yanlis okur).
param([Parameter(Mandatory = $true)] [string] $Exe)

$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W32Q {
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
}
"@

$env:GDPI3_TEST = "1"
$env:GDPI3_INSTANCE = ".quittest"
$failed = 0

function Start-App([string[]] $ArgList) {
    if ($ArgList) { return Start-Process -FilePath $Exe -ArgumentList $ArgList -PassThru }
    return Start-Process -FilePath $Exe -PassThru
}

function Wait-Exit([System.Diagnostics.Process] $p, [int] $ms) {
    return $p.WaitForExit($ms)
}

function Check([string] $name, [bool] $ok) {
    if ($ok) { Write-Host "  OK   $name" } else { Write-Host "  HATA $name"; $script:failed++ }
}

function Wait-Window([System.Diagnostics.Process] $p) {
    for ($i = 0; $i -lt 40; $i++) {
        $p.Refresh()
        if ($p.MainWindowHandle -ne [IntPtr]::Zero) { return $p.MainWindowHandle }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}

try {
    Write-Host "1) Tepsideki Cik (--quit ile ayni olay): pencere ve tepsi birlikte kapanir"
    $win = Start-App
    $tray = Start-App @("--tray")
    [void](Wait-Window $win)
    Start-Sleep -Milliseconds 800
    Check "pencere ve tepsi calisiyor" ((-not $win.HasExited) -and (-not $tray.HasExited))
    $q = Start-App @("--quit"); [void](Wait-Exit $q 3000)
    Check "pencere kapandi" (Wait-Exit $win 3000)
    Check "tepsi kapandi" (Wait-Exit $tray 3000)

    Write-Host "2) Cikistan sonra uygulama yeniden acilabilir (olay sifirlanir)"
    $win = Start-App
    [void](Wait-Window $win)
    Start-Sleep -Milliseconds 800
    Check "yeni pencere acik kaldi" (-not $win.HasExited)

    Write-Host "3) Ikinci acilis var olan pencereyi one getirir, yeni surec hemen biter"
    $second = Start-App
    Check "ikinci surec bitti" (Wait-Exit $second 3000)
    Check "ilk pencere hala acik" (-not $win.HasExited)

    Write-Host "4) Alt+F4 / WM_CLOSE: pencere solarak kapanir (tepside kal acikken tepsi kalir)"
    $tray = Start-App @("--tray")
    Start-Sleep -Milliseconds 800
    $h = Wait-Window $win
    [void][W32Q]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $closed = Wait-Exit $win 3000
    Check ("pencere kapandi ({0} ms)" -f $sw.ElapsedMilliseconds) $closed
    Start-Sleep -Milliseconds 500
    Check "tepsi calismaya devam ediyor" (-not $tray.HasExited)
    $q = Start-App @("--quit"); [void](Wait-Exit $q 3000)
    Check "tepsi --quit ile kapandi" (Wait-Exit $tray 3000)
}
finally {
    foreach ($p in @($win, $tray, $second)) {
        if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    }
    Remove-Item Env:GDPI3_TEST, Env:GDPI3_INSTANCE -ErrorAction SilentlyContinue
}

if ($failed -gt 0) { Write-Host "$failed kontrol basarisiz"; exit 1 }
Write-Host "Tum kontroller gecti"
