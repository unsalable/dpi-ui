#!/usr/bin/env bash
# Kendiliginden guncelleme, uctan uca ve gercek yayinla: eski bir surum (varsayilan v3.1.2-mac.3)
# GitHub'dan indirilip kurulur ve acilir; uygulama yeni surumu kendisi bulmali, imzasini gomulu
# anahtarla dogrulamali, kurup yeniden acilmali. Bekleme 3 dk.
#
#   bash perf/mac-update-e2e.sh [eski etiket] [beklenen surum] [cikti]
set -u
old="${1:-v3.1.2-mac.3}"
want="${2:-3.1.3}"
out="${3:-perf/out/mac-update}"
mkdir -p "$out"
app="/Applications/DPI UI.app"
data="$HOME/Library/Application Support/DPI UI"
fail=0
note() { echo "$*" | tee -a "$out/summary.txt"; }
bad() {
    note "HATA: $*"
    fail=1
}
version() { defaults read "$app/Contents/Info" CFBundleShortVersionString 2>/dev/null; }

rm -rf "$app" "$data"
tmp=$(mktemp -d)
curl -fsSL -o "$tmp/eski.zip" "https://github.com/unsalable/dpi-ui/releases/download/$old/DPI-UI-Mac.zip" || bad "eski surum indirilemedi"
ditto -x -k "$tmp/eski.zip" "$tmp/x"
mv "$tmp/x/DPI UI.app" "$app"
note "kurulan: $old (uygulama surumu $(version))"
open "$app"

for i in $(seq 1 90); do
    sleep 2
    v=$(version)
    if [ "$v" = "$want" ] && pgrep -f "$app/Contents/MacOS/DPI UI" >/dev/null; then
        note "guncellendi: $v ($((i * 2)) sn icinde), uygulama yeniden acik"
        break
    fi
done
[ "$(version)" = "$want" ] || bad "surum $want olmadi (simdi $(version))"
pgrep -fl "$app/Contents/MacOS/DPI UI" | tee -a "$out/summary.txt"
codesign --verify --deep --strict "$app" && note "imza saglam" || bad "imza bozuk"
[ -e "$app.eski" ] && bad "eski paket kaldi"
sleep 3
screencapture -x "$out/guncelleme.png"
note "--- hizmet gunlugu"
cat "$data/logs/service.log" | tee -a "$out/summary.txt"
osascript -e 'tell application "DPI UI" to quit' >/dev/null 2>&1
exit "$fail"
