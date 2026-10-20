//! Ayar ve ogrenilen alan adlarinin diske kaydi. Yazma atomiktir (gecici dosya +
//! yeniden adlandirma): elektrik kesilse bile yarim dosya kalmaz. Bozuk dosya
//! `.bad` uzantisiyla saklanip varsayilanlara donulur.

use std::path::Path;

use shared::config::AppConfig;
use shared::ipc::LearnedHost;
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

use crate::{info, warn};

const MAX_LEARNED: usize = 4000;

/// Veri klasorunu yalnizca SYSTEM ve Yoneticilerin yazabilecegi izinlerle olusturur.
pub fn ensure_data_dir() {
    let dir = shared::paths::service_data_dir();
    if dir.exists() {
        return;
    }
    // SYSTEM ve Yoneticiler: tam; Kullanicilar: okuma.
    let sddl: Vec<u16> = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FR;;;BU)\0".encode_utf16().collect();
    let path: Vec<u16> = dir.as_os_str().encode_wide_null();
    unsafe {
        let mut sd = std::ptr::null_mut();
        let ok = ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut());
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: if ok != 0 { sd } else { std::ptr::null_mut() },
            bInheritHandle: 0,
        };
        CreateDirectoryW(path.as_ptr(), &sa);
        if ok != 0 {
            windows_sys::Win32::Foundation::LocalFree(sd as _);
        }
    }
    let _ = std::fs::create_dir_all(&dir);
}

trait EncodeWideNull {
    fn encode_wide_null(&self) -> Vec<u16>;
}

impl EncodeWideNull for std::ffi::OsStr {
    fn encode_wide_null(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(Some(0)).collect()
    }
}

pub fn load_config() -> AppConfig {
    let path = shared::paths::config_file();
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<AppConfig>(&text) {
            Ok(c) => c.sanitized(),
            Err(e) => {
                warn!("Ayar dosyası bozuk ({e}); varsayılanlar kullanılıyor, eski dosya .bad olarak saklandı.");
                let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                AppConfig::default()
            }
        },
        Err(_) => {
            info!("Ayar dosyası yok; varsayılanlar kullanılıyor.");
            AppConfig::default()
        }
    }
}

pub fn save_config(cfg: &AppConfig) -> std::io::Result<()> {
    write_atomic(&shared::paths::config_file(), &serde_json::to_vec_pretty(cfg).map_err(std::io::Error::other)?)
}

pub fn load_learned() -> Vec<LearnedHost> {
    std::fs::read_to_string(shared::paths::learned_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<LearnedHost>>(&t).ok())
        .map(|mut v| {
            v.retain(|h| engine::scope::normalize(&h.host).is_some());
            v.truncate(MAX_LEARNED);
            v
        })
        .unwrap_or_default()
}

pub fn save_learned(list: &[LearnedHost]) -> std::io::Result<()> {
    let list = &list[..list.len().min(MAX_LEARNED)];
    write_atomic(&shared::paths::learned_file(), &serde_json::to_vec_pretty(list).map_err(std::io::Error::other)?)
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}
