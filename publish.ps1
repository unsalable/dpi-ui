# GoodbyeDPI UI 3 - yayin betigi
# 1) release derleme  2) dosyalari build\app'e toplama  3) simge  4) Inno Setup
# Kullanim: powershell -ExecutionPolicy Bypass -File publish.ps1
# Not: dosya bilerek yalnizca ASCII karakter icerir.

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

cargo build --release -p goodbyedpi3-service -p goodbyedpi3-ui
if ($LASTEXITCODE -ne 0) { throw "derleme basarisiz" }

$app = "build\app"
Remove-Item -Recurse -Force $app -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$app\licenses" | Out-Null
Copy-Item "target\release\GoodbyeDPI3.exe", "target\release\GoodbyeDPI3-Service.exe" $app
Copy-Item "third_party\windivert\2.2\WinDivert.dll", "third_party\windivert\2.2\WinDivert64.sys" $app
Copy-Item "third_party\windivert\2.2\LICENSE.txt" "$app\licenses\WinDivert-LICENSE.txt"

# Kurulum simgesi, uygulamanin kodla cizilen isaretinden uretilir.
$p = Start-Process -FilePath "$app\GoodbyeDPI3.exe" -ArgumentList "--write-icon", "build\app.ico" -PassThru -Wait
if (-not (Test-Path "build\app.ico")) { throw "simge uretilemedi" }

$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 bulunamadi" }
& $iscc /Q "installer\setup.iss"
if ($LASTEXITCODE -ne 0) { throw "kurulum paketi olusturulamadi" }

Get-ChildItem $app, "build\GoodbyeDPI-UI-3-Setup.exe" -File | ForEach-Object {
    "{0,-28} {1,8:N2} MB  {2}" -f $_.Name, ($_.Length / 1MB), (Get-FileHash $_.FullName -Algorithm SHA256).Hash.Substring(0, 16)
}
