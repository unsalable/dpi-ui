# DPI UI - Mac paketini imzalama
# GitHub'daki yayinin DPI-UI-Mac.zip dosyasini indirir, update-mac.json'u yerel imza anahtariyla
# imzalar ve yayina ekler. Mac uygulamasi guncellemeyi ancak bundan sonra kendiliginden kurar
# (imzasiz yayinda yalnizca elle kurulum onerir). Paket henuz yoksa "Mac yayini" is akisinin
# (GitHub'in Mac makinesi) bitmesini bekler. release.ps1 bunu yayindan sonra kendisi cagirir.
#
# Kullanim:
#   powershell -ExecutionPolicy Bypass -File release-mac.ps1 -Tag v3.2.0
#
# Imza anahtari varsayilan olarak %USERPROFILE%\.dpi-ui\release-signing.pk8 (depoda DEGIL).
# Not: dosya bilerek yalnizca ASCII karakter icerir.
param(
    [Parameter(Mandatory = $true)] [string] $Tag,
    [string] $Key = "$env:USERPROFILE\.dpi-ui\release-signing.pk8",
    [string] $Repo = "",
    [int] $WaitMinutes = 30
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if ($Tag -notmatch '^v\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') { throw "etiket bicimi gecersiz: $Tag" }
if (-not (Test-Path $Key)) { throw "imza anahtari yok: $Key" }
if (-not $Repo) { $Repo = (gh repo view --json nameWithOwner --jq .nameWithOwner) }
$version = $Tag.Substring(1)
$out = "build\release\mac-$version"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null

# Yayin henuz yokken gh stderr'e "release not found" yazar; PowerShell 5.1'de "Stop" altinda bu
# olumcul hata sayilir. Bekleme sirasinda yalnizca bu cagri icin "Continue".
function Get-ReleaseAssets {
    $old = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try { return @(gh release view $Tag --repo $Repo --json assets --jq '.assets[].name' 2>$null) }
    finally { $ErrorActionPreference = $old }
}

$deadline = (Get-Date).AddMinutes($WaitMinutes)
while ($true) {
    $assets = Get-ReleaseAssets
    if ($assets -contains 'DPI-UI-Mac.zip') { break }
    if ((Get-Date) -gt $deadline) { throw "DPI-UI-Mac.zip $WaitMinutes dakikada yayina eklenmedi; GitHub'da 'Mac yayini' is akisina bakin" }
    Write-Host "Mac paketi bekleniyor (GitHub'in Mac makinesi derliyor)..."
    Start-Sleep -Seconds 30
}

gh release download $Tag --repo $Repo --pattern 'DPI-UI-Mac.zip' --dir $out --clobber
if ($LASTEXITCODE -ne 0) { throw "Mac paketi indirilemedi" }
cargo build --release -p release-tool --quiet
& target\release\release-tool.exe sign-mac $Key "$out\DPI-UI-Mac.zip" $version $out
if ($LASTEXITCODE -ne 0) { throw "imzalama basarisiz" }
gh release upload $Tag "$out\update-mac.json" "$out\update-mac.json.sig" --repo $Repo --clobber
if ($LASTEXITCODE -ne 0) { throw "imza yayina eklenemedi" }
Write-Host "Mac paketi imzalandi: https://github.com/$Repo/releases/tag/$Tag"
