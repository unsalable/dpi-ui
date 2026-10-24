#!/usr/bin/env bash
# Roblox (Mac) uctan uca: oyun sistem proxy ayarina (PAC) bakmiyor, HTTPS_PROXY ortam degiskenine
# uyuyor (olculdu, 24.10.2026). DPI UI bagliyken kurulup acilan Roblox'un TCP baglantilarinin
# hepsi DPI UI'nin proxy'sinden gecmeli; Turkiye'de dogrudan baglanti DNS zehirlenmesi yuzunden
# "TlsVerificationFail" verir. DPI UI kapaninca degisken silinmeli.
#
#   bash perf/mac-roblox-test.sh ["build/mac/DPI UI.app"] [cikti]
set -u
app="${1:-build/mac/DPI UI.app}"
out="${2:-perf/out/mac-roblox}"
mkdir -p "$out"
data="$HOME/Library/Application Support/DPI UI"
fail=0
note() { echo "$*" | tee -a "$out/summary.txt"; }
bad() {
    note "HATA: $*"
    fail=1
}

rm -f "$data/ipc.sock"
open "$app"
for _ in $(seq 1 40); do
    [ -S "$data/ipc.sock" ] && break
    sleep 0.5
done
sleep 3
env_proxy=$(launchctl getenv HTTPS_PROXY)
note "HTTPS_PROXY: ${env_proxy:-yok}"
[ "$env_proxy" = "http://127.0.0.1:47813" ] || bad "baglaninca HTTPS_PROXY yazilmadi"

ver=$(curl -fsSL https://clientsettingscdn.roblox.com/v2/client-version/MacPlayer | sed 's/.*"clientVersionUpload":"\([^"]*\)".*/\1/')
note "Roblox: $ver"
curl -fsSL -o /tmp/roblox.dmg "https://setup.rbxcdn.com/mac/$ver-Roblox.dmg" || bad "Roblox indirilemedi"
hdiutil attach -nobrowse -readonly -mountpoint /tmp/rbx /tmp/roblox.dmg >/dev/null
installer=$(find /tmp/rbx -maxdepth 1 -name "*.app" | head -1)
rm -rf /tmp/RobloxInstaller.app
cp -R "$installer" /tmp/RobloxInstaller.app
hdiutil detach /tmp/rbx >/dev/null
open -n /tmp/RobloxInstaller.app
sleep 45
screencapture -x "$out/roblox.png"

: >"$out/lsof.txt"
for p in $(pgrep -f "RobloxPlayer"); do
    lsof -nP -a -p "$p" -iTCP >>"$out/lsof.txt" 2>/dev/null
done
via=$(grep -c "127.0.0.1:47813" "$out/lsof.txt")
direct=$(grep -- "->" "$out/lsof.txt" | grep -vc -- "->127.0.0.1:")
note "RobloxPlayer TCP: proxy uzerinden $via, dogrudan $direct"
[ "$via" -gt 0 ] || bad "Roblox proxy'yi kullanmadi"
[ "$direct" -eq 0 ] || bad "Roblox dogrudan baglanti kurdu"

pkill -f "Roblox.app|RobloxPlayer|RobloxInstaller"
osascript -e 'tell application "DPI UI" to quit' >/dev/null 2>&1
sleep 3
after=$(launchctl getenv HTTPS_PROXY)
note "cikistan sonra HTTPS_PROXY: ${after:-yok}"
[ -z "$after" ] || bad "cikista HTTPS_PROXY silinmedi"
# Kapanis temizligi her seferinde: uc tur ac (baglan) / Cmd+Q ile ayni yoldan kapat.
for round in 1 2 3; do
    rm -f "$data/ipc.sock"
    open "$app"
    for _ in $(seq 1 40); do
        [ "$(launchctl getenv HTTPS_PROXY)" = "http://127.0.0.1:47813" ] && break
        sleep 0.5
    done
    osascript -e 'tell application "DPI UI" to quit' >/dev/null 2>&1
    sleep 2
    left=$(launchctl getenv HTTPS_PROXY)
    pac=$(networksetup -getautoproxyurl "$(networksetup -listallnetworkservices | sed 1d | grep -v '^\*' | head -1)" | grep Enabled)
    note "tur $round: cikista HTTPS_PROXY=${left:-yok}, PAC $pac"
    [ -z "$left" ] || bad "tur $round: HTTPS_PROXY silinmedi"
    [ "$pac" = "Enabled: No" ] || bad "tur $round: PAC kapanmadi"
done

cp "$data/logs/service.log" "$out/" 2>/dev/null
exit "$fail"
