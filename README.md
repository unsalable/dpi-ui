# GoodbyeDPI UI 3

[![CI](https://github.com/unsalable/dpi-ui/actions/workflows/ci.yml/badge.svg)](https://github.com/unsalable/dpi-ui/actions/workflows/ci.yml)
[![Lisans: MIT](https://img.shields.io/badge/lisans-MIT-blue.svg)](LICENSE)

Türkiye için hafif, kararlı ve sade bir DPI atlatma uygulaması. GoodbyeDPI UI 2.x'in yerine
sıfırdan, Rust ile yazıldı; kendi DPI motorunu kullanır.

- **Neredeyse sıfır kaynak:** motor hizmeti ~1 MB, tepsi ~2 MB, pencere açıkken ~5 MB bellek;
  boşta işlemci kullanmaz. Pencere kapanınca süreç tamamen bellekten çıkar.
- **Motor bir Windows hizmeti:** arayüz kapalıyken de korur, çökerse kendiliğinden yeniden başlar
  ve trafik hiçbir zaman kesilmez (fail-open).
- **Akıllı liste:** yalnızca engelli sitelere uygulanır; açılmayan siteleri kendiliğinden öğrenir.
- **Şifreli DNS (DoH/DoT):** ağ ayarlarına dokunmadan; ulaşılamazsa sistem DNS'ine döner.
- İnternet sağlayıcısına özel hazır yöntemler (Türk Telekom, Superonline, Vodafone, TürkNet,
  Kablonet ve mobil hatlar), özel DNS, istisna listesi, eski sürüm ayarlarını otomatik aktarma.
- **Otomatik güncelleme:** yeni sürümler bu depodaki [Releases](https://github.com/unsalable/dpi-ui/releases)
  sayfasından gelir; paket yalnızca imzası doğrulanırsa kurulur.

## Kurulum

[Releases](https://github.com/unsalable/dpi-ui/releases) sayfasından en son
`GoodbyeDPI-UI-3-Setup.exe` dosyasını indirip çalıştırın. Kurulum yönetici izni ister (WinDivert
sürücüsü ve motor hizmeti için); sonrasında uygulama normal kullanıcı olarak çalışır.

## Güncellemeler nasıl çalışır

Motor hizmeti Windows açılınca, uygulama açılınca ve altı saatte bir bu deponun sürümlerine bakar.
Yeni sürüm varsa kurulum paketini indirir ve şu zinciri doğrular:

1. `update.json` (sürüm, boyut, SHA-256) uygulamaya gömülü Ed25519 açık anahtarıyla imzalı olmalı,
2. manifestteki sürüm yayın etiketiyle aynı ve yüklü sürümden yeni olmalı,
3. indirilen paketin boyutu ve SHA-256'sı manifestle birebir eşleşmeli.

Paket yalnızca SYSTEM'in yazabildiği bir klasörde durur. Kurulumu uygulama başlatır; Windows bir kez
yönetici onayı (UAC) ister, kurulum bitince uygulama kendiliğinden yeniden açılır. Ayarlardan
"Güncellemeleri otomatik yükle" kapatılırsa kurulum Hakkında sayfasından elle başlatılır.

## Derleme

Gereksinimler: Rust (MSVC), Visual Studio C++ araçları, Inno Setup 6.

```powershell
cargo test --workspace
powershell -ExecutionPolicy Bypass -File publish.ps1   # -> build\GoodbyeDPI-UI-3-Setup.exe
```

Arayüz testleri pencere açmadan bellekte çizer; `GDPI3_SHOTS=<klasör>` verilirse her sayfanın ve
bağlanma animasyonunun ekran görüntülerini PNG olarak yazar. Pencere/tepsi kapanma senaryoları:

```powershell
powershell -ExecutionPolicy Bypass -File perf\quit-test.ps1 -Exe target\release\GoodbyeDPI3.exe
powershell -ExecutionPolicy Bypass -File perf\tray-menu-test.ps1 -Exe target\release\GoodbyeDPI3.exe
```

Güncelleme denetimi (yönetici gerekmez; indirir ve doğrular, kurmaz):

```powershell
target\release\GoodbyeDPI3-Service.exe --check-update --as-version 3.0.0-alpha.0
```

Canlı motor testi (yönetici terminali, uygulamada bağlantı kesikken):

```powershell
& "C:\Program Files\GoodbyeDPI UI 3\GoodbyeDPI3-Service.exe" --probe
& "C:\Program Files\GoodbyeDPI UI 3\GoodbyeDPI3-Service.exe" --dns-bench
```

## Sürüm yayınlama

İmza anahtarı depoda değildir; yalnızca yayın yapan bilgisayarda
(`%USERPROFILE%\.dpi-ui\release-signing.pk8`) durur.

```powershell
powershell -ExecutionPolicy Bypass -File release.ps1 -Version 3.0.0-alpha.2 -Bump -Notes "Değişiklikler"
```

## Lisans

[MIT](LICENSE). Üçüncü taraf bileşenler ve lisansları: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
Paket yakalama: [WinDivert](https://github.com/basil00/WinDivert) (LGPL-3.0). DPI atlatma
teknikleri [GoodbyeDPI](https://github.com/ValdikSS/GoodbyeDPI) ve
[zapret](https://github.com/bol-van/zapret) projelerinin yayımladığı yöntemlere dayanır.

Arayüz [Slint](https://slint.dev) ile yapılmıştır:

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
