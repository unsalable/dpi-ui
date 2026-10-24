//! Ayar dosyasi (`config.json`). Yazma once gecici dosyaya, sonra yeniden adlandirma: yarim
//! kalan yazma eski ayari bozmaz.

use std::path::Path;

use shared::config::AppConfig;

pub fn load_config(dir: &Path) -> AppConfig {
    std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<AppConfig>(&t).ok())
        .unwrap_or_default()
        .sanitized()
}

pub fn save_config(dir: &Path, cfg: &AppConfig) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("config.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(cfg).map_err(std::io::Error::other)?)?;
    std::fs::rename(tmp, dir.join("config.json"))
}
