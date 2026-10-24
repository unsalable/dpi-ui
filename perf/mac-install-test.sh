#!/usr/bin/env bash
# Tek komutluk Mac kurulumunu (installer/mac/install.sh) gercek macOS'ta, GitHub'daki gercek
# yayina karsi dener: uygulama Uygulamalar'a kurulur, karantina isareti tasimaz (macOS'un
# "dogrulanamadi" uyarisi bu yuzden cikmaz) ve acilir. Karsilastirma icin ayni uygulama tarayici
# indirmesi gibi isaretlenince Gatekeeper'in ne dedigi de yazilir.
#
#   bash perf/mac-install-test.sh [cikti klasoru]
set -u
out="${1:-perf/out/mac-install}"
mkdir -p "$out"
app="/Applications/DPI UI.app"
fail=0
note() { echo "$*" | tee -a "$out/summary.txt"; }
bad() {
    note "HATA: $*"
    fail=1
}

rm -rf "$app"
if bash installer/mac/install.sh >"$out/install.log" 2>&1; then
    note "kurulum betigi: tamam"
else
    bad "kurulum betigi basarisiz"
fi
cat "$out/install.log"

[ -x "$app/Contents/MacOS/DPI UI" ] && note "uygulama yerinde: $app" || bad "uygulama kurulmadi"
if xattr "$app" 2>/dev/null | grep -q com.apple.quarantine; then
    bad "karantina isareti var (uyari cikar)"
else
    note "karantina isareti yok: macOS ilk acilista uyari gostermez"
fi
for _ in $(seq 1 30); do
    pgrep -f "$app/Contents/MacOS/DPI UI" >/dev/null && break
    sleep 0.5
done
pgrep -f "$app/Contents/MacOS/DPI UI" >/dev/null && note "uygulama acildi" || bad "uygulama acilmadi"
sleep 4
screencapture -x "$out/kurulum-sonrasi.png"

# Karsilastirma: tarayicidan indirilmis gibi isaretli kopya.
q=$(mktemp -d)/q.app
cp -R "$app" "$q"
xattr -w com.apple.quarantine "0081;$(printf '%x' "$(date +%s)");Safari;" "$q"
note "tarayici indirmesi gibi isaretli kopya: $(spctl -a -vv "$q" 2>&1 | head -2 | tr '\n' ' ')"

osascript -e 'tell application "DPI UI" to quit' >/dev/null 2>&1
sleep 2
rm -rf "$app" "$(dirname "$q")"
exit "$fail"
