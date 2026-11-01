//! Motor hizmeti surec basina bir kez baslar. Ilk baslatan arayuz (uygulama acildi) ya da VPN
//! hizmeti (telefon acildi, hizli ayar, her zaman acik VPN) olabilir; etkinlik kapanip yeniden
//! acilsa da motor ve baglanti surer.
//!
//! Baslatma kisa surer (ayar, gunluk, soket); baglanmak ise VPN hizmetini, yani Java'nin ana is
//! parcacigini bekler. Bu yuzden baglanma baslatmanin (OnceLock) disinda yapilir: ayni anda
//! motoru isteyen baska bir cagri (ornegin ana is parcacigindan) uzun sure beklemez.

use std::sync::{Arc, OnceLock};

use macservice::{Controller, Service};

static SERVICE: OnceLock<Option<Service>> = OnceLock::new();

/// Motoru (gerekirse) baslatir; ikinci deger bu cagrinin baslatip baslatmadigi.
fn init() -> (Option<Arc<Controller>>, bool) {
    let mut fresh = false;
    let service = SERVICE.get_or_init(|| {
        fresh = true;
        let _ = macservice::apk::INSTALLER.set(crate::bridge::install_apk);
        match macservice::start_with(|_| Box::new(crate::vpn::AndroidVpn::default()), false) {
            Ok(s) => Some(s),
            Err(e) => {
                macservice::error!("Motor hizmeti başlatılamadı: {e}");
                None
            }
        }
    });
    (service.as_ref().map(|s| s.controller().clone()), fresh)
}

/// Motoru (gerekirse) baslatir. Baslatilamadiysa `None`: arayuz "motor hizmeti kapali" gosterir.
pub fn ensure() -> Option<Arc<Controller>> {
    init().0
}

/// Uygulama acilisi: motor bu cagriyla basladiysa "Acilista otomatik baglan" uygulanir. VPN izni
/// henuz yoksa baglanilmaz (ilk acilista hata gostermesin; izin ilk dokunusta istenir). Etkinlik
/// yeniden acildiginda (motor zaten calisiyor) kullanicinin kestigi baglanti geri gelmez.
pub fn start_and_auto_connect() {
    let (Some(ctrl), true) = init() else { return };
    if ctrl.config().auto_connect && crate::bridge::vpn_permitted() {
        let _ = ctrl.connect();
    }
}

/// Calisan motor (baslatmaz).
pub fn running() -> Option<Arc<Controller>> {
    SERVICE.get().and_then(|s| s.as_ref()).map(|s| s.controller().clone())
}
