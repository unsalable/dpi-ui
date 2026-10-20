# GoodbyeDPI UI 3

Türkiye için hafif, kararlı ve animasyonlu bir DPI atlatma uygulaması. GoodbyeDPI UI 2.x'in
yerine sıfırdan yazıldı.

- **Neredeyse sıfır kaynak:** motor hizmeti ~1 MB, tepsi ~2 MB, pencere açıkken ~5 MB bellek;
  boşta işlemci kullanmaz. Pencere kapanınca süreç tamamen bellekten çıkar.
- **Motor bir Windows hizmeti:** UAC yalnızca kurulumda sorulur; arayüz kapalıyken de korur,
  çökerse kendiliğinden yeniden başlar ve trafik hiçbir zaman kesilmez (fail-open).
- **Akıllı liste:** yalnızca engelli sitelere uygulanır; açılmayan siteleri kendiliğinden öğrenir.
- **Şifreli DNS (DoH/DoT):** ağ ayarlarına dokunmadan; ulaşılamazsa sistem DNS'ine döner.
- İnternet sağlayıcısına özel hazır yöntemler (Türk Telekom, Superonline, Vodafone, TürkNet,
  Kablonet ve mobil hatlar), özel DNS, istisna listesi, eski sürüm ayarlarını otomatik aktarma.

## Kurulum

`GoodbyeDPI-UI-3-Setup.exe` dosyasını çalıştırın. Kurulum yönetici izni ister (WinDivert sürücüsü
ve motor hizmeti için); sonrasında uygulama normal kullanıcı olarak çalışır.

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

Canlı test (yönetici terminali, uygulamada bağlantı kesikken):

```powershell
& "C:\Program Files\GoodbyeDPI UI 3\GoodbyeDPI3-Service.exe" --probe
& "C:\Program Files\GoodbyeDPI UI 3\GoodbyeDPI3-Service.exe" --dns-bench
```

## Teşekkürler ve lisanslar

GoodbyeDPI (ValdikSS), zapret (bol-van) ve SplitWire-Turkey (cagritaskn) projelerinin fikirleri
üzerine kurulmuştur. Paket yakalama: [WinDivert](https://github.com/basil00/WinDivert) (LGPL-3.0).

Arayüz [Slint](https://slint.dev) ile yapılmıştır:

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
