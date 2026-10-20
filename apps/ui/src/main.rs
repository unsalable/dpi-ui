#![windows_subsystem = "windows"]

//! GoodbyeDPI UI 3 arayuzu.
//!   GoodbyeDPI3.exe          pencere (tek ornek; ikinci acilis var olani one getirir)
//!   GoodbyeDPI3.exe --tray   yalnizca tepsi simgesi (Slint yuklenmez)
//!   GoodbyeDPI3.exe --quit   acik pencereyi ve tepsiyi kapatir

mod client;
mod iconart;
mod prefs;
mod tray;
mod traymenu;
mod window;
mod winsys;
#[cfg(test)]
mod uitest;

slint::include_modules!();

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
        let text = winsys::wide(&format!("Pencere açılamadı: {e}"));
        let title = winsys::wide("GoodbyeDPI UI 3");
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONERROR) };
    }
}
