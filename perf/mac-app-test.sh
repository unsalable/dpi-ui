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

# Kendiliginden guncelleme (kurulum adimi): uygulama Uygulamalar'da calisirken isaretli bir "yeni
# surum" paketi kurulur; uygulama kapanir, yeni paket yerine konur ve uygulama yeniden acilir.
inst="/Applications/DPI UI.app"
rm -rf "$inst" "$inst.eski"
cp -R "$app" "$inst"
newdir=$(mktemp -d)
cp -R "$app" "$newdir/DPI UI.app"
echo deneme >"$newdir/DPI UI.app/Contents/Resources/guncelleme-denemesi"
codesign --force --deep --sign - "$newdir/DPI UI.app"
ditto -c -k --keepParent "$newdir/DPI UI.app" "$newdir/paket.zip"
rm -f "$data/ipc.sock"
open "$inst"
for _ in $(seq 1 40); do
    [ -S "$data/ipc.sock" ] && break
    sleep 0.5
done
sleep 2
oldpid=$(pgrep -f "^/Applications/DPI UI.app/Contents/MacOS/DPI UI" | head -1)
note "guncelleme oncesi PID: $oldpid"
if "$inst/Contents/MacOS/DPI UI" --install-update "$newdir/paket.zip" "$oldpid" 2>"$out/install.err"; then
    osascript -e 'tell application "DPI UI" to quit'
    newpid=""
    for _ in $(seq 1 60); do
        newpid=$(pgrep -f "^/Applications/DPI UI.app/Contents/MacOS/DPI UI" | head -1)
        if [ -f "$inst/Contents/Resources/guncelleme-denemesi" ] && [ -n "$newpid" ] && [ "$newpid" != "$oldpid" ]; then break; fi
        sleep 0.5
    done
    [ -f "$inst/Contents/Resources/guncelleme-denemesi" ] && note "yeni paket yerinde" || bad "yeni paket yerine konmadi"
    [ -n "$newpid" ] && [ "$newpid" != "$oldpid" ] && note "yeniden acildi: PID $newpid" || bad "uygulama yeniden acilmadi"
    [ -e "$inst.eski" ] && bad "eski paket silinmedi"
    codesign --verify --deep --strict "$inst" 2>>"$out/install.err" && note "kurulan paketin imzasi saglam" || bad "kurulan paketin imzasi bozuk"
    sleep 3
    screencapture -x "$out/9-guncelleme-sonrasi.png"
    osascript -e 'tell application "DPI UI" to quit'
    sleep 2
else
    bad "kurulum baslatilamadi: $(cat "$out/install.err")"
fi
rm -rf "$inst" "$newdir"

# Sayfa goruntuleri (Turkce): ayri veri klasoru, otomatik baglanti kapali; GDPI3_PAGE
# "derinlik:altsayfa" (0:0 ana ekran, 1:0 ayarlar, 2:1 yontem, 2:2 DNS, 2:3 siteler, 2:4 genel).
pages=$(mktemp -d)
printf '{"autoConnect": false, "language": "turkish"}' >"$pages/config.json"
shot() { # dosya, sayfa, tema (1 koyu, 2 acik), [yazi tipi]
    printf '{"theme": %s, "trayAtLogin": false, "autorunInitialized": true, "v1Checked": true, "language": "turkish"}' "$3" >"$pages/ui.json"
    if [ -n "${4:-}" ]; then export GDPI3_FONT="$4"; else unset GDPI3_FONT; fi
    GDPI3_DATA_DIR="$pages" GDPI3_TEST=1 GDPI3_PAGE="$2" "$app/Contents/MacOS/DPI UI" &
    local pid=$!
    sleep 4
    screencapture -x "$out/$1.png"
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    sleep 0.5
}
shot 3-ana-koyu 0:0 1
shot 4-ayarlar 1:0 2
shot 5-yontem 2:1 2
shot 6-dns 2:2 2
shot 7-siteler 2:3 2
shot 8-genel 2:4 2

# Yazi tipi denemesi (FONT_TRIALS="Ad1;Ad2"): buyuk harf isaretleri (Ö, İ, Ü) kirpiliyor mu?
if [ -n "${FONT_TRIALS:-}" ]; then
    IFS=';' read -ra fonts <<<"$FONT_TRIALS"
    for f in "${fonts[@]}"; do
        shot "font-$(echo "$f" | tr -cd 'A-Za-z0-9')" 2:1 2 "$f"
    done
fi
note "--- hizmet gunlugu"
cat "$out/service.log" 2>/dev/null | tee -a "$out/summary.txt"
exit "$fail"
