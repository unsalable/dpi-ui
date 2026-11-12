//! Kullaniciya ozel arayuz tercihleri: `%AppData%\GoodbyeDPI3\ui.json`.

use serde::{Deserialize, Serialize};
use shared::i18n::Language;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// 0 = sistem, 1 = koyu, 2 = acik
    pub theme: u8,
    pub close_to_tray: bool,
    pub tray_at_login: bool,
    /// v1 ayarlari aktarma teklifi bir kez yapilir.
    pub v1_checked: bool,
    /// Ilk acilista oturum acilisi kaydi yapildi mi?
    pub autorun_initialized: bool,
    /// Son bilinen dil (asil secim servisin ayarinda; pencere ve tepsi acilirken bunu kullanir).
    pub language: Language,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { theme: 0, close_to_tray: true, tray_at_login: true, v1_checked: false, autorun_initialized: false, language: Language::System }
    }
}

fn path() -> Option<std::path::PathBuf> {
    shared::paths::user_data_dir().map(|d| d.join("ui.json"))
}

pub fn load() -> Prefs {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .map(|mut p: Prefs| {
            p.theme = p.theme.min(2);
            p
        })
        .unwrap_or_default()
}

/// Motor hizmetinin diske kaydettigi ayar (`dir` = hizmetin veri klasoru; Windows'ta
/// `%ProgramData%\GoodbyeDPI3`, kullanicilar okuyabilir, yalnizca hizmet yazar). Hizmet
/// kapaliyken ekran bunu gosterir. Dosya yoksa hizmet de varsayilanlarla calisir; okunamazsa `None`.
pub fn saved_config(dir: &std::path::Path) -> Option<shared::config::AppConfig> {
    match std::fs::read_to_string(dir.join("config.json")) {
        Ok(text) => serde_json::from_str::<shared::config::AppConfig>(&text).ok().map(|c| c.sanitized()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(shared::config::AppConfig::default().sanitized()),
        Err(_) => None,
    }
}

/// Hizmetin ogrendigi siteler (`learned.json`, yalnizca Windows hizmeti yazar).
pub fn saved_learned(dir: &std::path::Path) -> Vec<shared::ipc::LearnedHost> {
    std::fs::read_to_string(dir.join("learned.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(p: &Prefs) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_vec_pretty(p) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, data).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}
