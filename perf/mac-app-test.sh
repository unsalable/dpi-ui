#!/usr/bin/env bash
# Mac uygulamasini gercek macOS'ta acar (GitHub'in Mac makinelerinde): hizmet soketi, otomatik
# baglanti ve sistem proxy'si, PAC, oturum acilisi kaydi; pencere ve menu cubugu ekran
# goruntuleri; "Cik" (Cmd+Q ile ayni yol) sonrasinda sistem ayarinin geri alindigi.
#
#   bash perf/mac-app-test.sh ["build/mac/DPI UI.app"] [cikti klasoru]
set -u
app="${1:-build/mac/DPI UI.app}"
out="${2:-perf/out/mac-app}"
mkdir -p "$out"
data="$HOME/Library/Application Support/DPI UI"
fail=0
note() { echo "$*" | tee -a "$out/summary.txt"; }
bad() {
    note "HATA: $*"
    fail=1
}
svc=$(networksetup -listallnetworkservices | sed 1d | grep -v '^\*' | head -1)
note "ag hizmeti: $svc"
networksetup -getautoproxyurl "$svc" >"$out/before.txt"

open "$app" || bad "uygulama acilamadi"
for _ in $(seq 1 40); do
    [ -S "$data/ipc.sock" ] && break
    sleep 0.5
done
[ -S "$data/ipc.sock" ] || bad "hizmet soketi olusmadi"
sleep 5
screencapture -x "$out/1-pencere.png"
screencapture -x -R 0,0,1024,32 "$out/2-menu-cubugu.png"

pac=$(networksetup -getautoproxyurl "$svc")
note "$pac"
grep -q "Enabled: Yes" <<<"$pac" || bad "otomatik baglanti sistem proxy'sini acmadi"
url=$(sed -n 's/^URL: //p' <<<"$pac")
curl -s "$url" -o "$out/proxy.pac" || bad "PAC indirilemedi ($url)"
grep -q '"discord.com": 1' "$out/proxy.pac" || bad "PAC'te hazir liste yok"
[ -f "$HOME/Library/LaunchAgents/com.dpi-ui.app.plist" ] && note "oturum acilisi kaydi: var" || bad "oturum acilisi kaydi yok"
cp "$HOME/Library/LaunchAgents/com.dpi-ui.app.plist" "$out/" 2>/dev/null

# Cik: NSApplication terminate (Cmd+Q ile ayni yol); sistem ayari geri alinmali.
osascript -e 'tell application "DPI UI" to quit' || bad "cik komutu gonderilemedi"
for _ in $(seq 1 20); do
    pgrep -f "DPI UI.app/Contents/MacOS" >/dev/null || break
    sleep 0.5
done
pgrep -f "DPI UI.app/Contents/MacOS" >/dev/null && bad "uygulama kapanmadi"
after=$(networksetup -getautoproxyurl "$svc")
note "cikistan sonra: $after"
grep -q "Enabled: No" <<<"$after" || bad "cikista sistem proxy ayari geri alinmadi"
[ -S "$data/ipc.sock" ] && note "uyari: soket dosyasi kaldi"

cp "$data/logs/service.log" "$out/" 2>/dev/null
cp "$data/config.json" "$out/" 2>/dev/null
note "--- hizmet gunlugu"
cat "$out/service.log" 2>/dev/null | tee -a "$out/summary.txt"
exit "$fail"
