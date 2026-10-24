#![windows_subsystem = "windows"]

//! DPI UI arayuzu.
//!   GoodbyeDPI3.exe          pencere (tek ornek; ikinci acilis var olani one getirir)
//!   GoodbyeDPI3.exe --tray   yalnizca tepsi simgesi (Slint yuklenmez)
//!   GoodbyeDPI3.exe --quit   acik pencereyi ve tepsiyi kapatir
//!
//! Mac'te tek surec: motor hizmeti (macservice) uygulamanin icinde calisir, pencere kapaninca
//! uygulama menu cubugunda kalir.
//!   "DPI UI" --write-iconset <klasor>   paketleme icin uygulama simgesi PNG'leri

mod client;
mod iconart;
#[cfg(target_os = "macos")]
mod macapp;
mod prefs;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod traymenu;
mod window;
// Platform katmani: Windows'ta Win32, Mac'te AppKit/komut satiri araclari. Pencere kodu
// ikisini de `winsys` adiyla cagirir.
#[cfg(windows)]
mod winsys;
#[cfg(target_os = "macos")]
#[path = "macsys.rs"]
mod winsys;
#[cfg(test)]
mod uitest;

slint::include_modules!();

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Yayin betigi kurulum simgesini buradan uretir.
    if let Some(i) = args.iter().position(|a| a == "--write-icon") {
        if let Some(path) = args.get(i + 1) {
            let ico = iconart::build_ico(&[16, 20, 24, 32, 40, 48, 64, 256], iconart::Mark::App);
            std::process::exit(if std::fs::write(path, ico).is_ok() { 0 } else { 1 });
        }
    }
    if args.iter().any(|a| a.eq_ignore_ascii_case("--tray")) {
        std::process::exit(tray::run());
    }
    // Acik pencereyi ve tepsiyi kapatir (kaldirma, guncelleme ve testler icin).
    if args.iter().any(|a| a.eq_ignore_ascii_case("--quit")) {
        winsys::signal_quit();
        return;
    }
    // Test modunda adlar ayridir; kullanicinin acik penceresiyle cakismaz.
    let Some(show_event) = winsys::single_instance(&winsys::instance_name("Local\\GoodbyeDPI3.Window")) else {
        return; // var olan pencere one getirildi
    };
    if let Err(e) = window::run(show_event) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
        let text = winsys::wide(&shared::t!("Pencere açılamadı: {e}", "Couldn't open the window: {e}"));
        let title = winsys::wide("DPI UI");
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR) };
    }
}

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--write-iconset") {
        let ok = args.get(i + 1).is_some_and(|dir| macapp::write_iconset(std::path::Path::new(dir)).is_ok());
        std::process::exit(if ok { 0 } else { 1 });
    }
    let service = match macservice::start() {
        Ok(s) => Some(s),
        Err(e) => {
            // Pencere yine acilir; "motor hizmeti kapali" gosterir ve gunluge yazar.
            eprintln!("Motor hizmeti başlatılamadı: {e}");
            None
        }
    };
    macapp::on_terminate({
        // Cmd+Q / oturum kapatma: sistem proxy ayari eski haline donsun.
        let ctrl = service.as_ref().map(|s| s.controller().clone());
        move || {
            if let Some(c) = &ctrl {
                c.disconnect();
            }
        }
    });
    if let Err(e) = window::run(()) {
        eprintln!("Pencere açılamadı: {e}");
    }
    if let Some(s) = service {
        s.shutdown();
    }
}
