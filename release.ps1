# GoodbyeDPI UI 3 - surum yayinlama
# 1) surum numarasini denetler (Cargo.toml ve installer\setup.iss ayni olmali)
# 2) publish.ps1 ile paketi derler   3) imzalar ve dogrular (tools\release)
# 4) etiketi gonderir ve GitHub'da surumu olusturur (on surumler "pre-release" isaretlenir)
#
# Kullanim:
#   powershell -ExecutionPolicy Bypass -File release.ps1 -Version 3.0.0-alpha.2 -Notes "Degisiklikler..."
#   -Bump   : surum numarasini dosyalara yazar ve commit eder (once)
#   -DryRun : GitHub'a hicbir sey gondermez, yalnizca paketi uretir ve imzalar
#
# Imza anahtari varsayilan olarak %USERPROFILE%\.dpi-ui\release-signing.pk8 (depoda DEGIL).
# Not: dosya bilerek yalnizca ASCII karakter icerir.
param(
    [Parameter(Mandatory = $true)] [string] $Version,
    [string] $Notes = "",
    [string] $Key = "$env:USERPROFILE\.dpi-ui\release-signing.pk8",
    [switch] $Bump,
    [switch] $DryRun
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') { throw "surum bicimi gecersiz: $Version" }
if (-not (Test-Path $Key)) { throw "imza anahtari yok: $Key" }

$cargo = Get-Content Cargo.toml -Raw
$iss = Get-Content installer\setup.iss -Raw
if ($Bump) {
    $cargo = $cargo -replace '(?m)^version = "[^"]+"', "version = `"$Version`""
    $iss = $iss -replace '#define AppVersion "[^"]+"', "#define AppVersion `"$Version`""
    [IO.File]::WriteAllText("$PSScriptRoot\Cargo.toml", $cargo)
    [IO.File]::WriteAllText("$PSScriptRoot\installer\setup.iss", $iss, [Text.UTF8Encoding]::new($true))
    cargo update --workspace --offline | Out-Null
    git add Cargo.toml Cargo.lock installer\setup.iss
    git commit -m "Surum $Version"
}
if ($cargo -notmatch "(?m)^version = `"$([regex]::Escape($Version))`"") { throw "Cargo.toml surumu $Version degil (-Bump kullanin)" }
if ($iss -notmatch "#define AppVersion `"$([regex]::Escape($Version))`"") { throw "setup.iss surumu $Version degil (-Bump kullanin)" }
if (-not $DryRun -and (git status --porcelain)) { throw "calisma agaci temiz degil; once commit edin" }

cargo test --workspace --quiet
if ($LASTEXITCODE -ne 0) { throw "testler basarisiz" }

powershell -ExecutionPolicy Bypass -File publish.ps1
if ($LASTEXITCODE -ne 0) { throw "paket olusturulamadi" }

$out = "build\release\$Version"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item "build\GoodbyeDPI-UI-3-Setup.exe" $out
cargo build --release -p release-tool --quiet
& target\release\release-tool.exe sign $Key "$out\GoodbyeDPI-UI-3-Setup.exe" $Version $out
if ($LASTEXITCODE -ne 0) { throw "imzalama basarisiz" }

if ($DryRun) {
    Write-Host "DryRun: paket ve imza $out icinde; GitHub'a gonderilmedi."
    return
}

$tag = "v$Version"
git tag -a $tag -m "GoodbyeDPI UI 3 $Version"
git push origin HEAD
git push origin $tag
$ghArgs = @("release", "create", $tag, "$out\GoodbyeDPI-UI-3-Setup.exe", "$out\update.json", "$out\update.json.sig",
    "--title", "GoodbyeDPI UI 3 $Version", "--notes", $(if ($Notes) { $Notes } else { "GoodbyeDPI UI 3 $Version" }))
if ($Version.Contains("-")) { $ghArgs += "--prerelease" }
& gh @ghArgs
if ($LASTEXITCODE -ne 0) { throw "GitHub surumu olusturulamadi" }
Write-Host "Yayinlandi: https://github.com/unsalable/dpi-ui/releases/tag/$tag"
