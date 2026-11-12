# Motor hizmeti kapaliyken ayarlar: pencere varsayilanlari degil hizmetin kaydettigi ayari
# gostermeli; o sirada degistirilen ayar hemen gorunmeli ve hizmet acilinca ona gonderilmeli.
# (Hata: "Windows ile baslat" kapaliyken bilgisayar acilinca hizmet calismaz; pencere anahtari
# acik gosteriyor, kapatmak "bagli degil" diyordu; baglaninca kendiliginden kapaniyordu.)
# Gercek servise dokunulmaz: ayar dosyasi gecici klasorde (GDPI3_DATA_DIR), hizmet once hic yok,
# sonra betigin actigi sahte pipe (GDPI3_PIPE). Pencere ekranda kisa sure acilir.
# Ornek: powershell -ExecutionPolicy Bypass -File perf\offline-settings-test.ps1 -Exe target\debug\GoodbyeDPI3.exe
# Not: dosya bilerek yalnizca ASCII karakter icerir (Windows PowerShell 5.1 BOM'suz UTF-8'i yanlis okur).
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [string] $OutDir = "perf\out"
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Runtime.InteropServices;

public static class W32O {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
}

// Sahte motor servisi: tek istemci; her istegin satirini saklar, servis gibi yanit verir.
public class OfflineFakeService {
    NamedPipeServerStream server;
    StreamReader reader;
    StreamWriter writer;
    readonly List<string> lines = new List<string>();
    readonly string config;

    public OfflineFakeService(string name, string config) {
        this.config = config;
        server = new NamedPipeServerStream(name, PipeDirection.InOut, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous, 65536, 65536);
    }

    public bool WaitClient(int ms) {
        IAsyncResult ar = server.BeginWaitForConnection(null, null);
        if (!ar.AsyncWaitHandle.WaitOne(ms)) return false;
        server.EndWaitForConnection(ar);
        UTF8Encoding utf8 = new UTF8Encoding(false);
        reader = new StreamReader(server, utf8);
        writer = new StreamWriter(server, utf8);
        writer.AutoFlush = true;
        Thread t = new Thread(Loop);
        t.IsBackground = true;
        t.Start();
        return true;
    }

    void Loop() {
        try {
            string line;
            while ((line = reader.ReadLine()) != null) {
                string kind = Regex.Match(line, "\"type\":\"(\\w+)\"").Groups[1].Value;
                string id = Regex.Match(line, "\"id\":(\\d+)").Groups[1].Value;
                lock (lines) lines.Add(line);
                string r;
                if (kind == "hello") r = "{\"type\":\"hello\",\"service_version\":\"9.9.9\",\"protocol\":1}";
                else if (kind == "getState") r = "{\"type\":\"state\",\"state\":{\"serviceVersion\":\"9.9.9\",\"engine\":{},\"config\":" + config + ",\"learned\":[],\"conflicts\":[],\"update\":{\"state\":\"idle\"}}}";
                else r = "{\"type\":\"ok\"}";
                writer.Write("{\"kind\":\"reply\",\"id\":" + id + ",\"response\":" + r + "}\n");
            }
        } catch (Exception) { }
    }

    // Bu turden ilk istegin satiri (yoksa null).
    public string Find(string kind) {
        lock (lines) {
            foreach (string l in lines) if (l.Contains("\"type\":\"" + kind + "\"")) return l;
        }
        return null;
    }

    public void Close() { try { server.Dispose(); } catch (Exception) { } }
}
"@

$failed = 0
function Check([string] $name, [bool] $ok) {
    if ($ok) { Write-Host "  OK   $name" } else { Write-Host "  HATA $name"; $script:failed++ }
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$data = Join-Path $env:TEMP "gdpi3-offline-$PID"
New-Item -ItemType Directory -Force -Path $data | Out-Null
# Kullanicinin kayitli ayari: Windows ile baslat ve otomatik baglan kapali (varsayilanlar acik).
[System.IO.File]::WriteAllText((Join-Path $data "config.json"), '{"startWithWindows":false,"autoConnect":false}')

# Pencereyi yakalar; "Windows ile baslat" satirindaki anahtarin yesil (acik) piksellerini sayar.
function Capture([IntPtr] $h, [string] $name) {
    $r = New-Object W32O+RECT; [void][W32O]::GetWindowRect($h, [ref]$r)
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    $bmp = New-Object System.Drawing.Bitmap $w, $hh
    $g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
    [void][W32O]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    $bmp.Save((Join-Path $OutDir "offline-$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    return $bmp
}
function GreenInToggleRow($bmp, [double] $s) {
    $n = 0
    for ($y = [int](96 * $s); $y -lt [int](124 * $s); $y++) {
        for ($x = [int](300 * $s); $x -lt [int](392 * $s); $x++) {
            $c = $bmp.GetPixel($x, $y)
            if ($c.G -gt 150 -and $c.R -lt 110 -and $c.B -lt 130) { $n++ }
        }
    }
    return $n
}

$pipeName = "GoodbyeDPI3.offlinetest.$PID"
$env:GDPI3_TEST = "1"; $env:GDPI3_INSTANCE = ".offlinetest"; $env:GDPI3_PIPE = "\\.\pipe\$pipeName"
$env:GDPI3_DATA_DIR = $data; $env:GDPI3_PAGE = "2:4"
$svc = $null
$p = Start-Process -FilePath $Exe -PassThru
try {
    for ($i = 0; $i -lt 100 -and $p.MainWindowHandle -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
    Start-Sleep -Milliseconds 2500
    # Ilk bulunan tutamac pencere cizilmeden onceki gecici bir pencere olabilir; yeniden alinir.
    $p.Refresh(); $h = $p.MainWindowHandle
    Check "pencere acildi" ($h -ne [IntPtr]::Zero)
    $s = [W32O]::GetDpiForWindow($h) / 96.0

    Write-Host "Hizmet kapali: kayitli ayar"
    $bmp = Capture $h "1-saved"
    $green = GreenInToggleRow $bmp $s; $bmp.Dispose()
    Check "Windows ile baslat kapali gorunuyor (yesil piksel: $green)" ($green -lt 20)

    Write-Host "Hizmet kapali: anahtara tiklama"
    $x = [int](200 * $s); $y = [int](110 * $s); $lp = [IntPtr](($y -shl 16) -bor $x)
    [void][W32O]::PostMessageW($h, 0x0200, [IntPtr]::Zero, $lp)
    [void][W32O]::PostMessageW($h, 0x0201, [IntPtr]1, $lp)
    [void][W32O]::PostMessageW($h, 0x0202, [IntPtr]::Zero, $lp)
    Start-Sleep -Milliseconds 700
    $bmp = Capture $h "2-edited"
    $green = GreenInToggleRow $bmp $s; $bmp.Dispose()
    Check "anahtar hemen acik gorunuyor (yesil piksel: $green)" ($green -ge 100)

    Write-Host "Hizmet acildi"
    # Hizmetin ayarinda baska bir alan da farkli: otomatik guncelleme yalnizca bildirim.
    $svc = New-Object OfflineFakeService($pipeName, '{"startWithWindows":false,"autoConnect":false,"updates":"notify"}')
    Check "pencere hizmete baglandi" ($svc.WaitClient(5000))
    $set = $null
    for ($i = 0; $i -lt 50 -and -not $set; $i++) { Start-Sleep -Milliseconds 100; $set = $svc.Find("setConfig") }
    Check "bekleyen degisiklik hizmete gonderildi" ($null -ne $set)
    if ($set) {
        Check "gonderilen ayarda Windows ile baslat acik" ($set.Contains('"startWithWindows":true'))
        Check "hizmetin kendi ayari korundu (updates=notify)" ($set.Contains('"updates":"notify"'))
        Check "otomatik baglan degismedi" ($set.Contains('"autoConnect":false'))
    }
    Start-Sleep -Milliseconds 500
    $bmp = Capture $h "3-connected"
    $green = GreenInToggleRow $bmp $s; $bmp.Dispose()
    Check "baglaninca anahtar acik kaldi (yesil piksel: $green)" ($green -ge 100)
    Write-Host "  goruntuler: $OutDir\offline-*.png"
}
finally {
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    if ($svc) { $svc.Close() }
    Remove-Item Env:GDPI3_TEST, Env:GDPI3_INSTANCE, Env:GDPI3_PIPE, Env:GDPI3_DATA_DIR, Env:GDPI3_PAGE -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
}

if ($failed -gt 0) { Write-Host "$failed kontrol basarisiz"; exit 1 }
Write-Host "Tum kontroller gecti"
