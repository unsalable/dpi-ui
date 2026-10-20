# Tepsi menusu imlec testi: menu penceresi normal ok imlecini kullanmali. Sinif imleci bos
# kalirsa Windows menunun ustunde bekleme (yukleniyor) imlecini gosteriyordu.
# Imlec oynatilmaz: menunun pencere sinifindaki imlec okunur ve sistemin ok imleciyle karsilastirilir.
# Kullanicinin acik uygulamasindan ayri adlarla calisir (GDPI3_INSTANCE).
# Ornek: powershell -ExecutionPolicy Bypass -File perf\tray-cursor-test.ps1 -Exe target\debug\GoodbyeDPI3.exe
# Not: dosya bilerek yalnizca ASCII karakter icerir (Windows PowerShell 5.1 BOM'suz UTF-8'i yanlis okur).
param(
    [Parameter(Mandatory = $true)] [string] $Exe
)

$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W32C {
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr LoadCursor(IntPtr inst, IntPtr id);
    [DllImport("user32.dll")] public static extern IntPtr GetClassLongPtr(IntPtr h, int index);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, IntPtr title);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out int pid);
    // Ayni sinif adli pencereler kullanicinin acik uygulamasinda da var: surece gore secilir.
    public static IntPtr FindOf(string cls, int pid) {
        IntPtr h = IntPtr.Zero;
        while ((h = FindWindowEx(IntPtr.Zero, h, cls, IntPtr.Zero)) != IntPtr.Zero) {
            int p; GetWindowThreadProcessId(h, out p);
            if (p == pid) return h;
        }
        return IntPtr.Zero;
    }
}
"@
$failed = 0
function Check([string] $name, [bool] $ok) {
    if ($ok) { Write-Host "  OK   $name" } else { Write-Host "  HATA $name"; $script:failed++ }
}
function Find-Menu {
    for ($i = 0; $i -lt 30; $i++) {
        $h = [W32C]::FindOf("GoodbyeDPI3TrayMenu", $tray.Id)
        if ($h -ne [IntPtr]::Zero) { return $h }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}
# Menu tepsiye sag tik bildirimi gonderilerek acilir. Arka plandaki bir surecten acilan menu one
# alinamayinca hemen kapanabilir; bu yuzden birkac kez denenir.
function Open-Menu {
    for ($try = 0; $try -lt 4; $try++) {
        $trayWnd = [W32C]::FindOf("GoodbyeDPI3Tray", $tray.Id)
        # WM_APP+1 (tepsi simgesi bildirimi), lParam = WM_CONTEXTMENU
        [void][W32C]::PostMessage($trayWnd, 0x8001, [IntPtr]::Zero, [IntPtr]0x007B)
        $h = Find-Menu
        if ($h -ne [IntPtr]::Zero) { return $h }
    }
    return [IntPtr]::Zero
}
$arrow = [W32C]::LoadCursor([IntPtr]::Zero, [IntPtr]32512)
$GCLP_HCURSOR = -12

$env:GDPI3_TEST = "1"; $env:GDPI3_INSTANCE = ".cursortest"
$tray = Start-Process -FilePath $Exe -ArgumentList "--tray" -PassThru
try {
    for ($i = 0; $i -lt 30 -and [W32C]::FindOf("GoodbyeDPI3Tray", $tray.Id) -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100 }
    $menu = Open-Menu
    Check "menu acildi" ($menu -ne [IntPtr]::Zero)
    $c = [W32C]::GetClassLongPtr($menu, $GCLP_HCURSOR)
    Write-Host "  sinif imleci: $c (ok imleci: $arrow)"
    Check "menu ok imlecini kullaniyor" ($c -eq $arrow)

    # Kapatip yeniden acinca da ayni (her sag tiklamada pencere yeniden olusturulur).
    [void][W32C]::PostMessage($menu, 0x0100, [IntPtr]27, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 400
    $menu = Open-Menu
    Check "menu yeniden acildi" ($menu -ne [IntPtr]::Zero)
    if ($menu -ne [IntPtr]::Zero) {
        Check "yeniden acilista da ok imleci" ([W32C]::GetClassLongPtr($menu, $GCLP_HCURSOR) -eq $arrow)
    }
}
finally {
    if (-not $tray.HasExited) { Stop-Process -Id $tray.Id -Force }
    Remove-Item Env:GDPI3_TEST, Env:GDPI3_INSTANCE -ErrorAction SilentlyContinue
}

if ($failed -gt 0) { Write-Host "$failed kontrol basarisiz"; exit 1 }
Write-Host "Tum kontroller gecti"
