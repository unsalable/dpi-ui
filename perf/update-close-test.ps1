# Sessiz guncelleme: arayuz surecleri servisten "kuruluyor" (installing) durumunu gorunce
# kendiliginden kapanmali (exe'leri degisecek; kurulumdan sonra servis yeniden acar).
# Gercek servise dokunulmaz: betik sahte bir servis pipe'i acar, arayuz GDPI3_TEST + GDPI3_PIPE
# ile ona baglanir. Pencere testi -Window ile calisir (ekranda kisa sure pencere acilir).
# Ornek: powershell -ExecutionPolicy Bypass -File perf\update-close-test.ps1 -Exe target\debug\GoodbyeDPI3.exe [-Window]
# Not: dosya bilerek yalnizca ASCII karakter icerir (Windows PowerShell 5.1 BOM'suz UTF-8'i yanlis okur).
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [switch] $Window
)

$ErrorActionPreference = "Stop"
Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;

// Sahte motor servisi: tek istemciyi kabul eder, isteklere servis gibi yanit verir.
// .NET Framework'te bir is parcacigi pipe'tan okurken baska birinin yazmasi bekler; bu yuzden
// okuma `stopAfter` istegi gelince biter, olaylar ondan sonra gonderilir.
public class FakeService {
    NamedPipeServerStream server;
    StreamReader reader;
    StreamWriter writer;
    readonly object gate = new object();
    readonly List<string> requests = new List<string>();
    readonly string update;
    readonly string stopAfter;
    readonly ManualResetEvent done = new ManualResetEvent(false);

    public FakeService(string name, string update, string stopAfter) {
        this.update = update;
        this.stopAfter = stopAfter;
        // Gercek servis gibi 64 KB tampon: tampon sifir olursa iki taraf da yazarken birbirini bekler.
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
                lock (requests) requests.Add(kind);
                string r;
                if (kind == "hello") r = "{\"type\":\"hello\",\"service_version\":\"9.9.9\",\"protocol\":1}";
                else if (kind == "getState") r = "{\"type\":\"state\",\"state\":{\"serviceVersion\":\"9.9.9\",\"engine\":{},\"config\":{},\"learned\":[],\"conflicts\":[],\"update\":" + update + "}}";
                else r = "{\"type\":\"ok\"}";
                Send("{\"kind\":\"reply\",\"id\":" + id + ",\"response\":" + r + "}");
                if (kind == stopAfter) break;
            }
        } catch (Exception) { }
        done.Set();
    }

    // Okuma bitti mi (beklenen istek geldi ya da istemci ayrildi)?
    public bool WaitDone(int ms) { return done.WaitOne(ms); }

    public bool Send(string json) {
        lock (gate) {
            try { writer.Write(json + "\n"); return true; } catch (Exception) { return false; }
        }
    }

    public bool Saw(string kind) { lock (requests) return requests.Contains(kind); }

    public string Seen() { lock (requests) return string.Join(",", requests.ToArray()); }

    public void Close() { try { server.Dispose(); } catch (Exception) { } }
}
"@
$failed = 0
function Check([string] $name, [bool] $ok) {
    if ($ok) { Write-Host "  OK   $name" } else { Write-Host "  HATA $name"; $script:failed++ }
}

$pipeName = "GoodbyeDPI3.updtest.$PID"
$installing = '{"state":"installing","version":"9.9.9"}'
$idle = '{"state":"idle"}'
$installingEvent = '{"kind":"event","event":{"type":"update","status":' + $installing + '}}'

$env:GDPI3_TEST = "1"; $env:GDPI3_INSTANCE = ".updtest"; $env:GDPI3_PIPE = "\\.\pipe\$pipeName"
try {
    # 1) Tepsi baglanir, guncelleme denetimi ister; "kuruluyor" gelince kapanir.
    Write-Host "Tepsi: kurulum basladiginda"
    $svc = New-Object FakeService($pipeName, $idle, "checkUpdate")
    $p = Start-Process -FilePath $Exe -ArgumentList "--tray" -PassThru
    try {
        Check "tepsi sahte servise baglandi" ($svc.WaitClient(5000))
        Check "acilista guncelleme denetimi istendi" ($svc.WaitDone(3000) -and $svc.Saw("checkUpdate"))
        Check "tepsi calisiyor" (-not $p.HasExited)
        [void]$svc.Send($installingEvent)
        Check "kurulum baslayinca tepsi kapandi" ($p.WaitForExit(3000))
        if ($p.HasExited) { Check "tepsi duzgun cikti (kod 0)" ($p.ExitCode -eq 0) }
    }
    finally {
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
        $svc.Close()
    }

    # 2) Kurulum surerken acilan tepsi hemen kapanir ve denetim istemez.
    Write-Host "Tepsi: kurulum surerken acilirsa"
    $svc = New-Object FakeService($pipeName, $installing, "")
    $p = Start-Process -FilePath $Exe -ArgumentList "--tray" -PassThru
    try {
        Check "tepsi sahte servise baglandi" ($svc.WaitClient(5000))
        Check "tepsi hemen kapandi" ($p.WaitForExit(3000))
        [void]$svc.WaitDone(1000)
        Check "denetim istenmedi" (-not $svc.Saw("checkUpdate"))
    }
    finally {
        if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
        $svc.Close()
    }

    # 3) Pencere: bildirim gosterir, kisa sure sonra kapanir.
    if ($Window) {
        Write-Host "Pencere: kurulum basladiginda"
        $svc = New-Object FakeService($pipeName, $idle, "checkUpdate")
        $p = Start-Process -FilePath $Exe -PassThru
        try {
            Check "pencere sahte servise baglandi" ($svc.WaitClient(15000))
            Check "pencere acilista guncelleme denetimi istedi" ($svc.WaitDone(8000) -and $svc.Saw("checkUpdate"))
            Check "pencere acik" (-not $p.HasExited)
            [void]$svc.Send($installingEvent)
            Check "pencere hemen kapanmadi (bildirim okunur)" (-not $p.WaitForExit(800))
            Check "pencere kapandi" ($p.WaitForExit(4000))
        }
        finally {
            if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
            $svc.Close()
        }
    }
}
finally {
    Remove-Item Env:GDPI3_TEST, Env:GDPI3_INSTANCE, Env:GDPI3_PIPE -ErrorAction SilentlyContinue
}

if ($failed -gt 0) { Write-Host "$failed kontrol basarisiz"; exit 1 }
Write-Host "Tum kontroller gecti"
