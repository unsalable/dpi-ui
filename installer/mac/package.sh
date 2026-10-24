#!/usr/bin/env bash
# Mac paketi: Apple Silicon + Intel tek uygulama (DPI UI.app) ve suruklenip birakilan .dmg.
# GitHub'in Mac makinelerinde calisir (Xcode araclari: lipo, iconutil, codesign, hdiutil).
#
#   bash installer/mac/package.sh            -> build/mac/DPI UI.app, build/DPI-UI-Mac.dmg
#
# Imza: Apple gelistirici hesabi olmadigi icin "ad-hoc" (yerel) imza. Indirilen uygulama ilk
# acilista "dogrulanamadi" der; Sistem Ayarlari > Gizlilik ve Guvenlik > "Yine de Ac" bir kez
# yeterli. Ad-hoc imza olmazsa macOS "hasarli" der ve bu secenek hic cikmaz.
set -euo pipefail
cd "$(dirname "$0")/../.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=build/mac
app="$out/DPI UI.app"
export MACOSX_DEPLOYMENT_TARGET=11.0

for t in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "$t" >/dev/null
    cargo build --release -p goodbyedpi3-ui --target "$t"
done

rm -rf "$out" && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/DPI UI" \
    target/aarch64-apple-darwin/release/GoodbyeDPI3 target/x86_64-apple-darwin/release/GoodbyeDPI3
sed "s/@VERSION@/$version/g" installer/mac/Info.plist >"$app/Contents/Info.plist"
printf 'APPL????' >"$app/Contents/PkgInfo"

"$app/Contents/MacOS/DPI UI" --write-iconset "$out/AppIcon.iconset"
iconutil -c icns -o "$app/Contents/Resources/AppIcon.icns" "$out/AppIcon.iconset"
cp LICENSE THIRD_PARTY_NOTICES.md "$app/Contents/Resources/"

codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
lipo -info "$app/Contents/MacOS/DPI UI"

# Disk goruntusu: uygulama + Uygulamalar kisayolu (surukle-birak kurulum).
stage="$out/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f build/DPI-UI-Mac.dmg
hdiutil create -volname "DPI UI" -srcfolder "$stage" -ov -format UDZO build/DPI-UI-Mac.dmg
ls -la build/DPI-UI-Mac.dmg
