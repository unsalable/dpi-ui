# Performans olcumu: bir sureci baslatir, RAM/CPU'yu ornekler, istenirse pencere goruntusu alir.
# Ornek:
#   powershell -File perf\measure.ps1 -Exe target\release\GoodbyeDPI3.exe -Seconds 10
#   powershell -File perf\measure.ps1 -Exe ... -EnvVars "GDPI_DEMO=1;X=2" -Shots "1,4.5,6"
# Not: dosya bilerek yalnizca ASCII karakter icerir (Windows PowerShell 5.1 BOM'suz UTF-8'i yanlis okur).
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [string] $EnvVars = "",
    [string] $Warmup = "2",
    [string] $Seconds = "10",
    [string] $Shots = "",
    [string] $OutDir = "perf\out",
    [string] $Label = "run"
)

$ErrorActionPreference = "Stop"
# Sayilar nokta ondalikla verilir; Turkce yerel ayarda virgul beklendigi icin degismez kulturle cozulur.
$inv = [Globalization.CultureInfo]::InvariantCulture
$warmupSec = [double]::Parse($Warmup, $inv)
$durationSec = [double]::Parse($Seconds, $inv)
$shotTimes = @($Shots.Split(",", [StringSplitOptions]::RemoveEmptyEntries) | ForEach-Object { [double]::Parse($_.Trim(), $inv) })
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W32 {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
}
"@

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$saved = @{}
foreach ($kv in ($EnvVars.Split(";", [StringSplitOptions]::RemoveEmptyEntries))) {
    $name, $value = $kv.Split("=", 2)
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
    [Environment]::SetEnvironmentVariable($name, $value)
}

$proc = Start-Process -FilePath $Exe -PassThru
foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }

function Save-Shot([System.Diagnostics.Process] $p, [string] $path) {
    $p.Refresh()
    $h = $p.MainWindowHandle
    if ($h -eq [IntPtr]::Zero) { Write-Host "pencere bulunamadi"; return }
    $r = New-Object W32+RECT
    [void][W32]::GetWindowRect($h, [ref]$r)
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [void][W32]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    Write-Host "goruntu: $path"
}

try {
    Start-Sleep -Milliseconds ([int]($warmupSec * 1000))
    $proc.Refresh()
    $start = Get-Date
    $cpu0 = $proc.TotalProcessorTime
    $samples = @()
    $pending = [System.Collections.Generic.List[double]]::new()
    foreach ($s in $shotTimes) { $pending.Add($s) }

    while (((Get-Date) - $start).TotalSeconds -lt $durationSec) {
        $proc.Refresh()
        if ($proc.HasExited) { throw "surec beklenmedik sekilde kapandi (kod $($proc.ExitCode))" }
        $samples += [pscustomobject]@{ Priv = $proc.PrivateMemorySize64; WS = $proc.WorkingSet64 }
        $elapsed = ((Get-Date) - $start).TotalSeconds
        for ($i = $pending.Count - 1; $i -ge 0; $i--) {
            if ($elapsed -ge $pending[$i]) {
                Save-Shot $proc (Join-Path $OutDir ("{0}-{1}s.png" -f $Label, $pending[$i].ToString("0.0", $inv)))
                $pending.RemoveAt($i)
            }
        }
        Start-Sleep -Milliseconds 250
    }

    $proc.Refresh()
    $wall = ((Get-Date) - $start).TotalSeconds
    $cpu = ($proc.TotalProcessorTime - $cpu0).TotalSeconds
    $mb = 1MB
    [pscustomobject]@{
        Label        = $Label
        Seconds      = [math]::Round($wall, 1)
        PrivAvgMB    = [math]::Round((($samples | Measure-Object Priv -Average).Average) / $mb, 1)
        PrivMaxMB    = [math]::Round((($samples | Measure-Object Priv -Maximum).Maximum) / $mb, 1)
        WSAvgMB      = [math]::Round((($samples | Measure-Object WS -Average).Average) / $mb, 1)
        CpuPctOfCore = [math]::Round(100 * $cpu / $wall, 2)
        Threads      = $proc.Threads.Count
    } | Format-List
}
finally {
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
}
