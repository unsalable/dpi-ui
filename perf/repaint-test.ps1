# Pencere cercevesi (DWM) sonradan degisince icerigin kaybolmadigini denetler.
# Gercek acilista servis durumu gelince tercihler ve cerceve yeniden uygulanir; test modunda bu
# adim GDPI3_REAPPLY_MS ile taklit edilir. Pencere ekrandan yakalanir, bos kalmasi gereken
# yerlerin uygulama renginde oldugu olculur. Goruntu PrintWindow ile alinir: ustteki baska
# pencereler (gorev degistirici vb.) olcumu bozmaz, pencere one zorlanmaz.
# Ornek: powershell -ExecutionPolicy Bypass -File perf\repaint-test.ps1 -Exe target\release\GoodbyeDPI3.exe
# Not: dosya bilerek yalnizca ASCII karakter icerir.
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [string] $OutDir = "perf\out"
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W32R {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
}
"@
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$env:GDPI3_TEST = "1"; $env:GDPI3_INSTANCE = ".repainttest"; $env:GDPI3_REAPPLY_MS = "1500"
$p = Start-Process -FilePath $Exe -PassThru
try {
    for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
    $h = $p.MainWindowHandle
    Start-Sleep -Milliseconds 3500
    $r = New-Object W32R+RECT; [void][W32R]::GetWindowRect($h, [ref]$r)
    $s = [W32R]::GetDpiForWindow($h) / 96.0
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
    [void][W32R]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    $path = Join-Path $OutDir "repaint.png"
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)

    # Olcum noktalari (mantiksal px): bos zemin (siyah) ve alttaki satir grubu (koyu gri).
    $bad = 0
    foreach ($pt in @(@(60, 120, "zemin"), @(340, 180, "zemin"), @(60, 450, "zemin"), @(200, 556, "satir"), @(200, 600, "satir"))) {
        $c = $bmp.GetPixel([int]($pt[0] * $s), [int]($pt[1] * $s))
        $max = [Math]::Max($c.R, [Math]::Max($c.G, $c.B))
        $ok = if ($pt[2] -eq "zemin") { $max -le 8 } else { $max -ge 20 -and $max -le 40 }
        Write-Host ("  {0,-6} ({1},{2}): R{3} G{4} B{5}  {6}" -f $pt[2], $pt[0], $pt[1], $c.R, $c.G, $c.B, $(if ($ok) { "OK" } else { "BOS / HATALI" }))
        if (-not $ok) { $bad++ }
    }
    $bmp.Dispose()
    Write-Host "  goruntu: $path"
    if ($bad -gt 0) { Write-Host "$bad nokta hatali: pencere icerigi kayboldu"; exit 1 }
    Write-Host "Pencere icerigi tam"
}
finally {
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    Remove-Item Env:GDPI3_TEST, Env:GDPI3_INSTANCE, Env:GDPI3_REAPPLY_MS -ErrorAction SilentlyContinue
}
