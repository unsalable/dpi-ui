//! Android platform katmani. Pencere kodu bunu Windows'taki gibi `winsys` adiyla cagirir:
//! tema, acilista baslatma, animasyon tercihi, adres acma, VPN izni.

use std::path::{Path, PathBuf};

use crate::bridge;

/// Sistem acik gorunumde mi?
pub fn system_prefers_light() -> bool {
    !bridge::night_mode()
}

/// Durum ve gezinme cubugundaki simgeler zemine gore (koyu zeminde acik simgeler).
pub fn set_window_theme(dark: bool) {
    bridge::set_dark_theme(dark);
}

/// Gelistirici seceneklerinde animasyonlar kapatildiysa (sure olcegi 0) gecisler aninda.
pub fn animations_enabled() -> bool {
    bridge::animations_enabled()
}

/// Motor uygulamanin icinde calisir. Henuz baslamadiysa (acilisin ilk aninda dokunuldu) arka
/// planda baslatilir; arayuz is parcacigi beklemez. Baslatilamadiysa uygulama yeniden acilmali.
pub fn start_service() -> Result<(), String> {
    if crate::engine_host::running().is_some() {
        return Ok(());
    }
    std::thread::Builder::new()
        .name("motor-baslat".into())
        .spawn(crate::engine_host::start_and_auto_connect)
        .map(|_| ())
        .map_err(|_| shared::t!("Motor hizmeti başlatılamadı. Uygulamayı kapatıp yeniden aç.", "The engine service couldn't start. Close the app and open it again."))
}

pub fn shell_open(target: &str) {
    bridge::open_url(target);
}

/// Gunlugun son kismini paylasma menusuyle gonderir (uygulama dosyalari baska uygulamalara kapali).
pub fn share_file_text(path: &Path, subject: &str) {
    const MAX: usize = 96 * 1024;
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let start = text.len().saturating_sub(MAX);
    let start = (start..text.len()).find(|&i| text.is_char_boundary(i)).unwrap_or(text.len());
    bridge::share_text(subject, &text[start..]);
}

/// Bagliyken kalici bildirimin metni.
pub fn show_connected(detail: &str) {
    bridge::update_notification(&shared::t!("Korumalı", "Protected"), detail);
}

pub fn vpn_permitted() -> bool {
    bridge::vpn_permitted()
}

pub fn request_vpn_permission(done: impl FnOnce(bool) + Send + 'static) {
    bridge::request_vpn_permission(done);
}

/// "Telefon acilinca baslat": Java'daki acilis alicisi bu dosyaya bakar.
fn autostart_flag() -> PathBuf {
    shared::paths::service_data_dir().join("autostart")
}

/// Telefon acilinca baslatmayi acar/kapatir. Basarisizsa false.
pub fn set_autorun(on: bool) -> bool {
    let flag = autostart_flag();
    if on { std::fs::write(&flag, b"1").is_ok() } else { std::fs::remove_file(&flag).is_ok() || !flag.exists() }
}

pub fn repair_autorun(on: bool) {
    if on != autostart_flag().exists() {
        set_autorun(on);
    }
}
