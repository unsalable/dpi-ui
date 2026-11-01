# DPI UI - Android paketini yayina ekleme
# APK'yi bu bilgisayarda derleyip imzalar (build-android.ps1), update-android.json'u yerel imza
# anahtariyla imzalar ve GitHub'daki yayina ekler. Yuklu Android uygulamalari guncellemeyi ancak
# bundan sonra bulur. release.ps1 bunu Windows yayinindan sonra kendisi cagirir.
#
# Kullanim:
#   powershell -ExecutionPolicy Bypass -File release-android.ps1 -Tag v3.2.0
#
# Imza anahtari varsayilan olarak %USERPROFILE%\.dpi-ui\release-signing.pk8 (depoda DEGIL).
# Not: dosya bilerek yalnizca ASCII karakter icerir.
param(
    [Parameter(Mandatory = $true)] [string] $Tag,
    [string] $Key = "$env:USERPROFILE\.dpi-ui\release-signing.pk8",
    [string] $Repo = ""
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if ($Tag -notmatch '^v\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') { throw "etiket bicimi gecersiz: $Tag" }
if (-not (Test-Path $Key)) { throw "imza anahtari yok: $Key" }
if (-not $Repo) { $Repo = (gh repo view --json nameWithOwner --jq .nameWithOwner) }
$version = $Tag.Substring(1)
# APK surumu Cargo.toml'dan gelir; etiketle ayni olmali (uygulama manifestteki surumu etiketle karsilastirir).
if ((Get-Content Cargo.toml -Raw) -notmatch "(?m)^version = `"$([regex]::Escape($version))`"") { throw "Cargo.toml surumu $version degil" }

powershell -ExecutionPolicy Bypass -File build-android.ps1
if ($LASTEXITCODE -ne 0) { throw "Android paketi derlenemedi" }

$out = "build\release\android-$version"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item "build\DPI-UI-Android.apk" $out
cargo build --release -p release-tool --quiet
& target\release\release-tool.exe sign-android $Key "$out\DPI-UI-Android.apk" $version $out
if ($LASTEXITCODE -ne 0) { throw "imzalama basarisiz" }
gh release upload $Tag "$out\DPI-UI-Android.apk" "$out\update-android.json" "$out\update-android.json.sig" --repo $Repo --clobber
if ($LASTEXITCODE -ne 0) { throw "Android paketi yayina eklenemedi" }
Write-Host "Android paketi yayina eklendi: https://github.com/$Repo/releases/tag/$Tag"
