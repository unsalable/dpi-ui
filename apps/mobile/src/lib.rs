//! DPI UI Android surumu.
//!
//! Masaustuyle ayni arayuz kodu (`apps/ui/src`: pencere, motor istemcisi, tercihler) buraya
//! dosya yoluyla dahil edilir; platform katmani (`winsys`) Android'inkidir. Motor Mac'teki gibi
//! uygulamanin icinde calisir (`macservice`); trafigi proxy'ye sistem proxy ayari yerine VPN
//! getirir (`vpn`). Java tarafi (`apps/android`) yalnizca Android'in zorunlu kildigi parcalar:
//! etkinlik, VPN hizmeti, bildirim, acilista baslatma.
//!
//! Bilgisayarda yalnizca arayuz testleri derlenir (`uitest`).

slint::include_modules!();

#[cfg(target_os = "android")]
mod bridge;
#[cfg(target_os = "android")]
#[path = "../../ui/src/client.rs"]
mod client;
#[cfg(target_os = "android")]
mod engine_host;
#[cfg(target_os = "android")]
#[path = "../../ui/src/prefs.rs"]
mod prefs;
#[cfg(target_os = "android")]
mod vpn;
#[cfg(target_os = "android")]
#[path = "../../ui/src/window.rs"]
mod window;
#[cfg(target_os = "android")]
#[path = "androidsys.rs"]
mod winsys;
#[cfg(test)]
mod uitest;

/// NativeActivity'nin cagirdigi giris noktasi (her etkinlik acilisinda yeni bir is parcaciginda).
/// Java tarafi `Native.init`'i bundan once cagirmistir (veri klasoru, dil, Java sinifi).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    use slint::android::android_activity::{MainEvent, PollEvent};

    // Motor ayri is parcaciginda baslar: acilista otomatik baglanirken VPN hizmetini bekler, o da
    // Java'nin ana is parcaciginda calisir; Java ise etkinlik olaylarinda bu is parcacigini
    // (android_main) bekler. Pencere motora baglanana kadar kendisi yeniden dener.
    let _ = std::thread::Builder::new().name("motor-baslat".into()).spawn(engine_host::start_and_auto_connect);
    let listener = |event: &PollEvent<'_>| {
        // Sistem koyu/acik gorunume gecti (etkinlik yeniden kurulmaz: manifest uiMode).
        if let PollEvent::Main(MainEvent::ConfigChanged { .. }) = event {
            window::system_theme_changed();
        }
    };
    if let Err(e) = slint::android::init_with_event_listener(app, listener) {
        macservice::error!("Arayüz başlatılamadı: {e}");
        return;
    }
    if let Err(e) = window::run(()) {
        macservice::error!("Pencere açılamadı: {e}");
    }
}
