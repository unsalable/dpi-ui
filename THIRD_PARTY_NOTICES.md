# Üçüncü taraf bileşenler

GoodbyeDPI UI 3'ün kendi kodu [MIT lisanslıdır](LICENSE). Uygulamayla birlikte dağıtılan ya da
derlenirken içine giren bileşenler ve lisansları:

| Bileşen | Kullanım | Lisans |
| --- | --- | --- |
| [WinDivert 2.2](https://github.com/basil00/WinDivert) (basil00) | Paket yakalama sürücüsü (`WinDivert.dll`, `WinDivert64.sys`), değiştirilmeden, dinamik bağlanır | LGPL-3.0 (GPL-2.0 ile çift lisans); metni `licenses\WinDivert-LICENSE.txt` |
| [Slint](https://slint.dev) | Arayüz kütüphanesi | Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0; metni `licenses\Slint-Royalty-free-2.0.md` (atıf: README'deki "Made with Slint" rozeti) |
| [rustls](https://github.com/rustls/rustls), [ring](https://github.com/briansmith/ring) | Şifreli DNS (DoH/DoT) | Apache-2.0 / MIT / ISC |
| [webpki-roots](https://github.com/rustls/webpki-roots) | Kök sertifikalar | CDLA-Permissive-2.0 |
| serde, serde_json, windows-sys, windows-service, winit ve diğer Rust kütüphaneleri | Çeşitli | MIT ve/veya Apache-2.0 (ayrıntı: her paketin kendi `Cargo.toml` dosyası) |

## Fikirler ve teknikler

Motor kendi kodumuzdur; DPI atlatma teknikleri açık kaynak topluluğunun yıllar içinde yayımladığı
yöntemlere dayanır: [GoodbyeDPI](https://github.com/ValdikSS/GoodbyeDPI) (Apache-2.0) ve
[zapret](https://github.com/bol-van/zapret) (MIT). Bu projelerin kaynak kodu uygulamaya
kopyalanmamıştır.
