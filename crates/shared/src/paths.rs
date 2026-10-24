//! Dosya konumlari.
//!
//! Windows: servis verileri `%ProgramData%\GoodbyeDPI3`, arayuz tercihleri `%AppData%\GoodbyeDPI3`.
//! macOS: motor uygulamanin icinde calistigi icin ikisi de `~/Library/Application Support/DPI UI`.

use std::path::PathBuf;

#[cfg(windows)]
const DIR: &str = "GoodbyeDPI3";
#[cfg(not(windows))]
const DIR: &str = "DPI UI";

/// Servisin verileri: `%ProgramData%\GoodbyeDPI3` (yalnizca servis/yonetici yazar).
pub fn service_data_dir() -> PathBuf {
    // Testler ve gelistirme icin gercek klasore dokunmadan calisabilmek.
    if let Some(dir) = std::env::var_os("GDPI3_DATA_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    let base = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    #[cfg(not(windows))]
    let base = app_support_base();
    base.join(DIR)
}

/// `~/Library/Application Support` (diger Unix'lerde `~/.local/share`).
#[cfg(not(windows))]
fn app_support_base() -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    if cfg!(target_os = "macos") { home.join("Library").join("Application Support") } else { home.join(".local").join("share") }
}

pub fn config_file() -> PathBuf {
    service_data_dir().join("config.json")
}

pub fn learned_file() -> PathBuf {
    service_data_dir().join("learned.json")
}

/// Indirilen guncelleme: yalnizca servis (SYSTEM) yazar, kullanicilar okuyabilir.
pub fn update_dir() -> PathBuf {
    service_data_dir().join("update")
}

pub fn update_installer() -> PathBuf {
    update_dir().join(crate::update::INSTALLER_NAME)
}

/// Servisin kurulumu baslatmadan once yazdigi not: hangi oturumlarda arayuz acikti. Kurulumdan
/// sonra baslayan servis bunu okur, arayuzu yeniden acar ve dosyayi siler.
pub fn update_relaunch_file() -> PathBuf {
    update_dir().join("relaunch.json")
}

pub fn log_dir() -> PathBuf {
    service_data_dir().join("logs")
}

/// Kullanicinin arayuz tercihleri: `%AppData%\GoodbyeDPI3`.
pub fn user_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(DIR));
    #[cfg(not(windows))]
    return Some(service_data_dir());
}

/// v1'in ayar dosyasi (yalnizca Windows'ta vardi).
pub fn v1_settings_file() -> Option<PathBuf> {
    #[cfg(windows)]
    return std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("GoodbyeDPI-UI").join("settings.json"));
    #[cfg(not(windows))]
    return None;
}

/// Calisan exe'nin klasoru (WinDivert dosyalari burada).
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}
