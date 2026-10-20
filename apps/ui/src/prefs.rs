//! Kullaniciya ozel arayuz tercihleri: `%AppData%\GoodbyeDPI3\ui.json`.

use serde::{Deserialize, Serialize};

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
}

impl Default for Prefs {
    fn default() -> Self {
        Self { theme: 0, close_to_tray: true, tray_at_login: true, v1_checked: false, autorun_initialized: false }
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
