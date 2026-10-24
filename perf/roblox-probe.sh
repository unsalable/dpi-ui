#!/usr/bin/env bash
# Roblox (Mac) hangi proxy ayarina uyuyor? GitHub'in Mac makinesinde dpi-proxy calisirken Roblox
# uc ayarla acilir ve isteklerinin proxy'den gecip gecmedigi proxy kaydindan okunur:
#   pac     : sistem "otomatik proxy" (PAC) - Mac surumunun bugunku yontemi
#   manual  : sistem "guvenli web proxy'si" (elle girilen HTTPS proxy)
#   env     : launchctl setenv HTTPS_PROXY / https_proxy / ALL_PROXY
#
#   bash perf/roblox-probe.sh [dpi-proxy] [cikti]
set -u
exe="${1:-target/release/dpi-proxy}"
out="${2:-perf/out/roblox}"
mkdir -p "$out"
port=47813
note() { echo "$*" | tee -a "$out/summary.txt"; }
svc=$(networksetup -listallnetworkservices | sed 1d | grep -v '^\*' | head -1)

"$exe" --port "$port" --pac smart --verbose >"$out/proxy.out" 2>"$out/proxy.log" &
ppid=$!
sleep 1

ver=$(curl -fsSL https://clientsettingscdn.roblox.com/v2/client-version/MacPlayer | sed 's/.*"clientVersionUpload":"\([^"]*\)".*/\1/')
note "Roblox surumu: $ver"
curl -fsSL -o /tmp/roblox.dmg "https://setup.rbxcdn.com/mac/$ver-Roblox.dmg" || note "dmg indirilemedi"
hdiutil attach -nobrowse -readonly -mountpoint /tmp/rbx /tmp/roblox.dmg >/dev/null
ls -la /tmp/rbx | tee -a "$out/summary.txt"
app=$(find /tmp/rbx -maxdepth 1 -name "*.app" | head -1)
cp -R "$app" /tmp/Roblox.app
hdiutil detach /tmp/rbx >/dev/null
note "uygulama: $app ($(defaults read /tmp/Roblox.app/Contents/Info CFBundleExecutable 2>/dev/null))"

reset() {
    networksetup -setautoproxystate "$svc" off
    networksetup -setsecurewebproxystate "$svc" off
    networksetup -setwebproxystate "$svc" off
    for v in HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy; do launchctl unsetenv "$v"; done
}

run_case() { # ad
    local name="$1"
    local before
    before=$(wc -l <"$out/proxy.log")
    open -n /tmp/Roblox.app
    sleep 25
    screencapture -x "$out/$name.png"
    pgrep -fl "Roblox" >"$out/$name-ps.txt"
    for p in $(pgrep -f "Roblox.app|RobloxPlayer"); do lsof -nP -a -p "$p" -iTCP >>"$out/$name-lsof.txt" 2>/dev/null; done
    pkill -f "Roblox.app|RobloxPlayer"
    sleep 2
    tail -n +"$((before + 1))" "$out/proxy.log" >"$out/$name-proxy.txt"
    local hits
    hits=$(grep -ci "roblox\|rbx" "$out/$name-proxy.txt")
    note "$name: proxy'den gecen Roblox baglantisi = $hits; dogrudan TCP (lsof) = $(grep -c TCP "$out/$name-lsof.txt" 2>/dev/null)"
    grep -i "roblox\|rbx" "$out/$name-proxy.txt" | head -5 | tee -a "$out/summary.txt"
}

reset
networksetup -setautoproxyurl "$svc" "http://127.0.0.1:$port/proxy.pac"
run_case pac

reset
networksetup -setsecurewebproxy "$svc" 127.0.0.1 "$port"
networksetup -setwebproxy "$svc" 127.0.0.1 "$port"
run_case manual

reset
for v in HTTPS_PROXY https_proxy HTTP_PROXY http_proxy ALL_PROXY all_proxy; do launchctl setenv "$v" "http://127.0.0.1:$port"; done
run_case env

reset
kill "$ppid" 2>/dev/null
ls ~/Applications /Applications 2>/dev/null | grep -i roblox | tee -a "$out/summary.txt"
exit 0
