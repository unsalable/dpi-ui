//! Ayar ve ogrenilen alan adlarinin diske kaydi. Yazma atomiktir (gecici dosya +
//! yeniden adlandirma): elektrik kesilse bile yarim dosya kalmaz. Bozuk dosya
//! `.bad` uzantisiyla saklanip varsayilanlara donulur.

use std::path::Path;

use shared::config::AppConfig;
use shared::ipc::LearnedHost;
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES, SetFileSecurityW};
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

use crate::{info, warn};

const MAX_LEARNED: usize = 4000;

/// SYSTEM ve Yoneticiler: tam; Kullanicilar: okuma.
const DATA_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FR;;;BU)";
/// Guncelleme klasoru ve paketi: Kullanicilar okur ve CALISTIRIR ama yazamaz. Paketi
/// kullanicinin oturumundaki arayuz baslatir; okuma izni tek basina yetmez (Windows
/// "erisim engellendi" der). 0x1200a9 = okuma + calistirma.
const UPDATE_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";
const UPDATE_FILE_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x1200a9;;;BU)";

/// Veri klasorunu yalnizca SYSTEM ve Yoneticilerin yazabilecegi izinlerle olusturur.
pub fn ensure_data_dir() {
    let dir = shared::paths::service_data_dir();
    if !dir.exists() {
        create_dir_with(&dir, DATA_SDDL);
    }
}

/// Guncelleme klasorunu olusturur ya da izinlerini duzeltir (eski surumler klasoru
/// calistirma izni olmadan olusturuyordu).
pub fn ensure_update_dir(dir: &Path) {
    if dir.exists() {
        apply_sddl(dir, UPDATE_SDDL);
    } else {
        create_dir_with(dir, UPDATE_SDDL);
    }
}

/// Indirilen paketin izinleri: kullanicilar calistirabilir, degistiremez.
pub fn secure_update_file(path: &Path) -> bool {
    apply_sddl(path, UPDATE_FILE_SDDL)
}

fn create_dir_with(dir: &Path, sddl: &str) {
    let sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
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
    let _ = std::fs::create_dir_all(dir);
}

/// Var olan dosya/klasorun DACL'ini korumali (ust klasorden miras almayan) olarak degistirir.
pub fn apply_sddl(path: &Path, sddl: &str) -> bool {
    let sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let p: Vec<u16> = path.as_os_str().encode_wide_null();
    unsafe {
        let mut sd = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) == 0 {
            return false;
        }
        let ok = SetFileSecurityW(p.as_ptr(), DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, sd) != 0;
        windows_sys::Win32::Foundation::LocalFree(sd as _);
        ok
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn elevated() -> bool {
        use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return true;
            }
            let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut len = 0u32;
            let ok = GetTokenInformation(token, TokenElevation, &mut e as *mut _ as *mut _, size_of::<TOKEN_ELEVATION>() as u32, &mut len);
            windows_sys::Win32::Foundation::CloseHandle(token);
            ok == 0 || e.TokenIsElevated != 0
        }
    }

    /// Guncelleme paketi standart kullanici yetkisiyle calistirilabilmeli (3.0.0'a kadar
    /// klasor yalnizca okuma izni veriyordu ve Windows "erisim engellendi" diyordu).
    #[test]
    fn users_can_run_the_update_package() {
        let root = std::env::temp_dir().join(format!("gdpi3-acl-{}", std::process::id()));
        let dir = root.join("update");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("probe.exe");
        std::fs::copy(r"C:\Windows\System32\whoami.exe", &exe).unwrap();

        assert!(apply_sddl(&dir, UPDATE_SDDL));
        assert!(secure_update_file(&exe));
        let run = || std::process::Command::new(&exe).output();
        assert!(run().is_ok_and(|o| o.status.success()), "yeni izinlerle paket calismali");

        // Eski izin (yalnizca okuma): standart kullanici calistiramaz. Yonetici olarak
        // calisan testte (CI) bu karsilastirma anlamsiz oldugu icin atlanir.
        assert!(apply_sddl(&exe, "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;BU)"));
        if !elevated() {
            let err = run().expect_err("eski izinle calismamali");
            assert_eq!(err.raw_os_error(), Some(5), "{err}");
        }

        // Temizlik: sahip olarak izni acip sil.
        let open = "D:(A;OICI;FA;;;WD)";
        apply_sddl(&exe, open);
        apply_sddl(&dir, open);
        let _ = std::fs::remove_dir_all(&root);
    }
}
