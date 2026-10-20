# Simge durumuna kucultup geri getirince pencere icerigi tam mi? (Windows kucultmede icerigi
# atar; uygulama geri gelince tamamini yeniden cizmeli. 3.0.2'ye kadar yalnizca degisen yerler
# ciziliyordu ve pencere bos gorunuyordu.)
# Ornek: powershell -ExecutionPolicy Bypass -File perf\minimize-test.ps1 -Exe target\release\GoodbyeDPI3.exe
# Not: dosya bilerek yalnizca ASCII karakter icerir.
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [string] $OutDir = "perf\out",
    [int] $Cycles = 3
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W32N {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
}
"@
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# Pencerenin kendi icerigini (PrintWindow) olcer: alttaki satir koyu gri, bos zemin siyah olmali.
function Test-Content([IntPtr] $h, [string] $name) {
    $r = New-Object W32N+RECT; [void][W32N]::GetWindowRect($h, [ref]$r)
    $s = [W32N]::GetDpiForWindow($h) / 96.0
    $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
    $g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
    [void][W32N]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    $row = $bmp.GetPixel([int](200 * $s), [int](556 * $s))
    $bmp.Save((Join-Path $OutDir "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    $rowOk = $row.R -ge 20 -and $row.R -le 40
    Write-Host ("  {0,-10} satir R{1} G{2} B{3}  {4}" -f $name, $row.R, $row.G, $row.B, $(if ($rowOk) { "OK" } else { "BOS" }))
    return $rowOk
}

$env:GDPI3_TEST = "1"; $env:GDPI3_INSTANCE = ".minimizetest"
$p = Start-Process -FilePath $Exe -PassThru
$failed = 0
try {
    for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
    $h = $p.MainWindowHandle
    Start-Sleep -Milliseconds 1500
    if (-not (Test-Content $h "acilis")) { $failed++ }
    for ($c = 1; $c -le $Cycles; $c++) {
        [void][W32N]::ShowWindow($h, 6)   # SW_MINIMIZE
        Start-Sleep -Milliseconds 800
        [void][W32N]::ShowWindow($h, 9)   # SW_RESTORE
        Start-Sleep -Milliseconds 800
        if (-not (Test-Content $h "geri-$c")) { $failed++ }
    }
}
finally {
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    Remove-Item Env:GDPI3_TEST, Env:GDPI3_INSTANCE -ErrorAction SilentlyContinue
}
if ($failed -gt 0) { Write-Host "$failed olcum basarisiz: pencere icerigi kayboldu"; exit 1 }
Write-Host "Tum olcumler gecti"
