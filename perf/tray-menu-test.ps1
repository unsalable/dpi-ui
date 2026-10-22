# Tepsi menusu canli testi: menuyu acar, ekrandan yakalar, uzerine gelme / Esc / Cik dener.
# Kullanicinin acik uygulamasindan ayri adlarla calisir (GDPI3_INSTANCE); baglanti anahtarina dokunmaz.
# Ornek: powershell -ExecutionPolicy Bypass -File perf\tray-menu-test.ps1 -Exe target\debug\GoodbyeDPI3.exe -OutDir perf\out
# Not: dosya bilerek yalnizca ASCII karakter icerir (Windows PowerShell 5.1 BOM'suz UTF-8'i yanlis okur).
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [string] $OutDir = "perf\out"
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W32M {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, IntPtr title);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
}
"@
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$failed = 0
function Check([string] $name, [bool] $ok) {
    if ($ok) { Write-Host "  OK   $name" } else { Write-Host "  HATA $name"; $script:failed++ }
}
function Find-Menu {
    for ($i = 0; $i -lt 30; $i++) {
        $h = [W32M]::FindWindow("GoodbyeDPI3TrayMenu", [IntPtr]::Zero)
        if ($h -ne [IntPtr]::Zero) { return $h }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}
function Shot([IntPtr] $h, [string] $name) {
    $r = New-Object W32M+RECT; [void][W32M]::GetWindowRect($h, [ref]$r)
    $m = 24
    $w = $r.R - $r.L + 2 * $m; $hh = $r.B - $r.T + 2 * $m
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L - $m, $r.T - $m, 0, 0, $bmp.Size)
    $g.Dispose()
    $path = Join-Path $OutDir $name
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    Write-Host "  goruntu: $path"
}
function LParam([int] $x, [int] $y) { return [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF)) }

$env:GDPI3_TEST = "1"; $env:GDPI3_INSTANCE = ".menutest"; $env:GDPI3_TRAY_MENU = "1"
$tray = Start-Process -FilePath $Exe -ArgumentList "--tray" -PassThru
try {
    $menu = Find-Menu
    Check "menu acildi" ($menu -ne [IntPtr]::Zero)
    $s = [W32M]::GetDpiForWindow($menu) / 96.0
    Start-Sleep -Milliseconds 300
    Shot $menu "tray-menu-live.png"

    # Secili satir: asagi ok iki kez -> "Uygulamayi ac" (fare menude olmadigi icin fareyle
    # gelinen vurguyu Windows'un "fare cikti" iletisi hemen siler). Vurgu yavasca belirir.
    [void][W32M]::PostMessage($menu, 0x0100, [IntPtr]0x28, [IntPtr]::Zero)
    [void][W32M]::PostMessage($menu, 0x0100, [IntPtr]0x28, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 350
    Shot $menu "tray-menu-live-hover.png"

    # Esc menuyu kapatir, tepsi calismaya devam eder.
    [void][W32M]::PostMessage($menu, 0x0100, [IntPtr]27, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
    Check "Esc menuyu kapatti" ([W32M]::FindWindow("GoodbyeDPI3TrayMenu", [IntPtr]::Zero) -eq [IntPtr]::Zero)
    Check "tepsi calisiyor" (-not $tray.HasExited)
}
finally {
    if (-not $tray.HasExited) { Stop-Process -Id $tray.Id -Force }
}

# Cik: yeni bir tepsi, menude "Cik" satirina tiklanir; surec kapanmali.
$tray = Start-Process -FilePath $Exe -ArgumentList "--tray" -PassThru
try {
    $menu = Find-Menu
    $s = [W32M]::GetDpiForWindow($menu) / 96.0
    $lp = LParam ([int](130 * $s)) ([int](148 * $s))
    [void][W32M]::PostMessage($menu, 0x0200, [IntPtr]::Zero, $lp)
    [void][W32M]::PostMessage($menu, 0x0202, [IntPtr]::Zero, $lp)
    Check "Cik tepsiyi kapatti" ($tray.WaitForExit(3000))
}
finally {
    if (-not $tray.HasExited) { Stop-Process -Id $tray.Id -Force }
    Remove-Item Env:GDPI3_TEST, Env:GDPI3_INSTANCE, Env:GDPI3_TRAY_MENU -ErrorAction SilentlyContinue
}

if ($failed -gt 0) { Write-Host "$failed kontrol basarisiz"; exit 1 }
Write-Host "Tum kontroller gecti"
