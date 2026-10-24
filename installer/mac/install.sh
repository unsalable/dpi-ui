#!/bin/bash
# DPI UI - Mac kurulumu, tek komut:
#
#   curl -fsSL https://raw.githubusercontent.com/unsalable/dpi-ui/main/installer/mac/install.sh | bash
#
# macOS'un "doğrulanamadı" uyarısı çıkmaz: macOS bu uyarıyı yalnızca tarayıcının "internetten
# indirildi" (karantina) diye işaretlediği dosyalarda gösterir; curl ile inen dosyada bu işaret
# yoktur. Ücretli Apple geliştirici hesabı gerekmez. Sonraki güncellemeleri uygulama kendisi
# indirip imzasını doğrulayarak kurar.
#
# Ne yapar: GitHub'daki en yeni Mac paketini (DPI-UI-Mac.zip) indirir, SHA-256'sını aynı yayındaki
# manifestle karşılaştırır, açık DPI UI'yi kapatır, uygulamayı Uygulamalar klasörüne koyar ve açar.
set -euo pipefail

REPO="unsalable/dpi-ui"
APP="DPI UI.app"
say() { printf '%s\n' "$*"; }
fail() {
    printf 'Kurulum durdu: %s\n' "$*" >&2
    exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "bu betik yalnızca macOS içindir"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "DPI UI'nin en yeni Mac sürümü aranıyor..."
# Sitenin yayın akışı (API'nin saatlik sınırına takılmaz); yayınlar yeniden eskiye sıralı.
curl -fsSL "https://github.com/$REPO/releases.atom" -o "$tmp/releases.atom" || fail "GitHub'a ulaşılamadı"
base=""
for tag in $(grep -o "releases/tag/[^\"<]*" "$tmp/releases.atom" | sed 's|releases/tag/||' | awk '!seen[$0]++'); do
    url="https://github.com/$REPO/releases/download/$tag"
    # Mac paketi olan ilk yayın en yenisidir.
    if curl -fsSLI -o /dev/null "$url/DPI-UI-Mac.zip"; then
        base="$url"
        break
    fi
done
[ -n "$base" ] || fail "yayınlarda Mac paketi bulunamadı"
zip_url="$base/DPI-UI-Mac.zip"
say "Sürüm: ${base##*/}"

curl -fsSL "$base/update-mac.json" -o "$tmp/update-mac.json" || fail "manifest indirilemedi"
curl -fL --progress-bar "$zip_url" -o "$tmp/DPI-UI-Mac.zip" || fail "paket indirilemedi"
want=$(grep -o '"sha256": *"[0-9a-f]*"' "$tmp/update-mac.json" | sed 's/.*"\([0-9a-f]*\)"/\1/' || true)
got=$(shasum -a 256 "$tmp/DPI-UI-Mac.zip" | cut -d' ' -f1)
{ [ -n "$want" ] && [ "$want" = "$got" ]; } || fail "indirilen paket yayındaki özetle eşleşmiyor"

# Açıksa kapat: kapanırken sistem proxy ayarı eski hâline döner.
if pgrep -f "$APP/Contents/MacOS/DPI UI" >/dev/null; then
    say "Açık DPI UI kapatılıyor..."
    osascript -e 'tell application "DPI UI" to quit' >/dev/null 2>&1 || true
    for _ in $(seq 1 30); do
        pgrep -f "$APP/Contents/MacOS/DPI UI" >/dev/null || break
        sleep 0.3
    done
fi

# Uygulamalar klasörü yönetici kullanıcıya yazılabilir; standart kullanıcıda kendi klasörü.
dest=/Applications
if [ ! -w "$dest" ]; then
    dest="$HOME/Applications"
    mkdir -p "$dest"
fi
ditto -x -k "$tmp/DPI-UI-Mac.zip" "$tmp/app" || fail "paket açılamadı"
[ -x "$tmp/app/$APP/Contents/MacOS/DPI UI" ] || fail "pakette uygulama yok"
rm -rf "${dest:?}/$APP"
mv "$tmp/app/$APP" "$dest/"
xattr -dr com.apple.quarantine "$dest/$APP" 2>/dev/null || true
say "Kuruldu: $dest/$APP"
open "$dest/$APP"
say "DPI UI açıldı; sağ üstte menü çubuğunda dünya simgesi görünür."
