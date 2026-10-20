//! Dosya konumlari.

use std::path::PathBuf;

const DIR: &str = "GoodbyeDPI3";

/// Servisin verileri: `%ProgramData%\GoodbyeDPI3` (yalnizca servis/yonetici yazar).
pub fn service_data_dir() -> PathBuf {
    // Testler ve gelistirme icin gercek klasore dokunmadan calisabilmek.
    if let Some(dir) = std::env::var_os("GDPI3_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    base.join(DIR)
}

pub fn config_file() -> PathBuf {
    service_data_dir().join("config.json")
}

pub fn learned_file() -> PathBuf {
    service_data_dir().join("learned.json")
}

pub fn log_dir() -> PathBuf {
    service_data_dir().join("logs")
}

/// Kullanicinin arayuz tercihleri: `%AppData%\GoodbyeDPI3`.
pub fn user_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(DIR))
}

/// v1'in ayar dosyasi.
pub fn v1_settings_file() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("GoodbyeDPI-UI").join("settings.json"))
}

/// Calisan exe'nin klasoru (WinDivert dosyalari burada).
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}
