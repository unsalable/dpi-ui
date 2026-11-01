# DPI UI - Android paketi (APK)
# 1) Rust kutuphanesini (apps/mobile -> libdpiui.so) telefon mimarileri icin derler (cargo ndk)
# 2) Java parcalariyla birlikte Gradle ile paketler ve imzalar
# 3) build\DPI-UI-Android.apk olarak yazar ve imzayi denetler
#
# Kullanim:
#   powershell -ExecutionPolicy Bypass -File build-android.ps1
#   -Debug : hata ayiklama anahtariyla, yalnizca x86_64 (emulator) icin hizli deneme paketi
#   -Abi arm64-v8a,x86_64 : yayin paketine emulator mimarisini de koyar (varsayilan yalnizca telefonlar)
#
# Gerekenler: Android SDK (platform 36, build-tools), NDK, JDK 17, cargo-ndk, Rust hedefleri
# aarch64-linux-android ve x86_64-linux-android.
# Yayin imzasi: %USERPROFILE%\.dpi-ui\android-release.jks ve sifresi android-release.pass (depoda
# DEGIL; kaybolursa yuklu uygulamalar yeni surumu kabul etmez, yedekleyin). Yoksa olusturur.
# Not: dosya bilerek yalnizca ASCII karakter icerir.
param(
    [switch] $Debug,
    [string[]] $Abi = @("arm64-v8a")
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk" }
if (-not (Test-Path "$env:ANDROID_HOME\platforms")) { throw "Android SDK bulunamadi: $env:ANDROID_HOME" }
if (-not $env:ANDROID_NDK_HOME) {
    $ndk = Get-ChildItem "$env:ANDROID_HOME\ndk" -Directory | Sort-Object Name | Select-Object -Last 1
    if (-not $ndk) { throw "Android NDK bulunamadi ($env:ANDROID_HOME\ndk)" }
    $env:ANDROID_NDK_HOME = $ndk.FullName
}
if (-not $env:JAVA_HOME) {
    $jdk = Get-ChildItem "C:\Program Files\Eclipse Adoptium" -Directory -Filter "jdk-17*" -ErrorAction SilentlyContinue | Select-Object -Last 1
    if ($jdk) { $env:JAVA_HOME = $jdk.FullName }
}
$platform = Get-ChildItem "$env:ANDROID_HOME\platforms" -Directory | Sort-Object Name | Select-Object -Last 1
$env:ANDROID_JAR = "$($platform.FullName)\android.jar"
$buildTools = Get-ChildItem "$env:ANDROID_HOME\build-tools" -Directory | Sort-Object Name | Select-Object -Last 1

$keyDir = "$env:USERPROFILE\.dpi-ui"
$keyStore = "$keyDir\android-release.jks"
$keyPass = "$keyDir\android-release.pass"
if (-not $Debug) {
    if (-not (Test-Path $keyStore)) {
        Write-Host "Android imza anahtari yok; olusturuluyor: $keyStore"
        New-Item -ItemType Directory -Force $keyDir | Out-Null
        $bytes = New-Object byte[] 24
        [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
        $pass = [Convert]::ToBase64String($bytes) -replace '[+/=]', 'x'
        # keytool uyarilarini stderr'e yazar; PowerShell 5.1 "Stop" altinda bunu hata sayar.
        $ErrorActionPreference = "Continue"
        & "$env:JAVA_HOME\bin\keytool.exe" -genkeypair -keystore $keyStore -alias dpi-ui -keyalg RSA -keysize 4096 `
            -validity 18250 -storepass $pass -keypass $pass -dname "CN=DPI UI, O=unsalable" 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        if ($LASTEXITCODE -ne 0) { throw "imza anahtari olusturulamadi" }
        [IO.File]::WriteAllText($keyPass, $pass)
        Write-Host "UYARI: $keyStore ve $keyPass dosyalarini yedekleyin."
    }
    $env:DPI_UI_ANDROID_KEY_PASS = (Get-Content $keyPass -Raw).Trim()
}

# 1) Rust. cargo ndk yalnizca libdpiui.so'yu degil bagimliliklarin .so'larini da kopyalar
# (tun2proxy); bu yuzden cikti once ayri klasore yazilir, sonra yalnizca libdpiui.so alinir.
# 32-bit ARM (armeabi-v7a) yok: Slint'in Skia cizicisinin o mimari icin hazir derlemesi yayimlanmiyor.
# Yayin paketi varsayilan olarak yalnizca telefonlar (arm64): x86_64 kopyasi paketi iki katina cikarir.
$abis = if ($Debug) { @("x86_64") } else { @($Abi | ForEach-Object { $_ -split "," }) }
$ndkOut = "target\android-out"
Remove-Item -Recurse -Force $ndkOut -ErrorAction SilentlyContinue
$ndkArgs = @("ndk")
foreach ($abi in $abis) { $ndkArgs += @("-t", $abi) }
$ndkArgs += @("-o", $ndkOut, "build", "-p", "dpi-ui-mobile")
if (-not $Debug) { $ndkArgs += "--release" }
$env:CARGO_TARGET_DIR = "target\android"
& cargo @ndkArgs
if ($LASTEXITCODE -ne 0) { throw "Rust derlemesi basarisiz" }
Remove-Item Env:\CARGO_TARGET_DIR

$jni = "apps\android\app\src\main\jniLibs"
Remove-Item -Recurse -Force $jni -ErrorAction SilentlyContinue
foreach ($abi in $abis) {
    New-Item -ItemType Directory -Force "$jni\$abi" | Out-Null
    Copy-Item "$ndkOut\$abi\libdpiui.so" "$jni\$abi\libdpiui.so"
}

# 2) Gradle.
$task = if ($Debug) { "assembleDebug" } else { "assembleRelease" }
Push-Location apps\android
try {
    & .\gradlew.bat $task --no-daemon --quiet
    if ($LASTEXITCODE -ne 0) { throw "Gradle derlemesi basarisiz" }
} finally {
    Pop-Location
}
$variant = if ($Debug) { "debug" } else { "release" }
$apk = "apps\android\app\build\outputs\apk\$variant\app-$variant.apk"
New-Item -ItemType Directory -Force build | Out-Null
$outApk = if ($Debug) { "build\DPI-UI-Android-debug.apk" } else { "build\DPI-UI-Android.apk" }
Copy-Item $apk $outApk -Force

# 3) Imza denetimi: yayin paketi hata ayiklama anahtariyla cikmasin.
$certs = & "$($buildTools.FullName)\apksigner.bat" verify --print-certs $outApk
if ($LASTEXITCODE -ne 0) { throw "APK imzasi dogrulanamadi" }
if (-not $Debug -and ($certs -join "`n") -match "CN=Android Debug") { throw "yayin paketi hata ayiklama anahtariyla imzalanmis" }
$size = [math]::Round((Get-Item $outApk).Length / 1MB, 1)
Write-Host "Android paketi hazir: $outApk ($size MB)"
