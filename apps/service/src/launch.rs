//! Servisin baslattigi surecler.
//!
//! - Guncelleme kurulumu: servis SYSTEM olarak calistigi icin paketi sessizce kurar; Windows
//!   onay (UAC) sormaz, ekranda pencere acilmaz. Kurulum hizmeti durdurur: paket servisten
//!   bagimsiz bir surec olarak baslatilir ki hizmet kapaninca yarida kalmasin.
//! - Arayuzu yeniden acma: kurulumdan sonra tepsi ve pencere, kullanicinin oturumunda ve
//!   kullanicinin kendi (yonetici olmayan) yetkisiyle baslatilir.

use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::RemoteDesktop::WTSQueryUserToken;
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, CreateProcessW,
    PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTF_FORCEOFFFEEDBACK, STARTUPINFOW, WaitForSingleObject,
};

/// Baslatilan surecin tutamaci (dusurulunce kapanir).
pub struct Process(HANDLE);

unsafe impl Send for Process {}

impl Process {
    /// Surec `ms` icinde biterse `true`.
    pub fn wait(&self, ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.0, ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Sessiz kurulumun komut satiri. `/UPDATE=service`: kurulum arayuzu kendisi acmaz (SYSTEM
/// olarak acardi); yeni servis acar. Kurulum gunlugu servis gunluklerinin yanina yazilir.
pub fn installer_command(installer: &Path, log: &Path) -> String {
    format!(
        "\"{}\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /NOCANCEL /UPDATE=service /LOG=\"{}\"",
        installer.display(),
        log.display()
    )
}

/// Sureci olusturur. Servis bir is nesnesindeyse (job) cocuk surec ondan ayrilir ki servis
/// dururken kapanmasin; is nesnesi ayrilmaya izin vermiyorsa ayrilmadan yeniden denenir.
fn spawn(mut create: impl FnMut(PROCESS_CREATION_FLAGS, &mut PROCESS_INFORMATION) -> bool, flags: PROCESS_CREATION_FLAGS) -> Result<Process, u32> {
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = create(flags | CREATE_BREAKAWAY_FROM_JOB, &mut pi) || {
        let first = unsafe { GetLastError() };
        // 5 = erisim engellendi: is nesnesi ayrilmaya izin vermiyor.
        first == 5 && create(flags, &mut pi)
    };
    if !ok {
        return Err(unsafe { GetLastError() });
    }
    unsafe { CloseHandle(pi.hThread) };
    Ok(Process(pi.hProcess))
}

/// Kurulum paketini sessizce baslatir (beklemez).
pub fn run_installer(installer: &Path, log: &Path) -> Result<Process, String> {
    let mut cmd = wide(&installer_command(installer, log));
    let app = wide(&installer.display().to_string());
    let dir = installer.parent().map(|d| wide(&d.display().to_string()));
    spawn(
        |flags, pi| unsafe {
            let mut si: STARTUPINFOW = std::mem::zeroed();
            si.cb = size_of::<STARTUPINFOW>() as u32;
            CreateProcessW(
                app.as_ptr(),
                cmd.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                flags,
                std::ptr::null(),
                dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                &si,
                pi,
            ) != 0
        },
        CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP,
    )
    .map_err(|e| format!("kurulum başlatılamadı (Windows hata kodu {e})"))
}

/// `exe args` komutunu verilen oturumdaki kullanici olarak calistirir. Oturumda kimse yoksa
/// (oturum kapandi) hata doner. Imlec "yukleniyor" durumuna gecmez.
pub fn run_as_session_user(session: u32, exe: &Path, args: &str) -> Result<(), String> {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { WTSQueryUserToken(session, &mut token) } == 0 {
        return Err(format!("{session}. oturumun kullanıcısı bulunamadı (hata {})", unsafe { GetLastError() }));
    }
    let mut env: *mut core::ffi::c_void = std::ptr::null_mut();
    if unsafe { CreateEnvironmentBlock(&mut env, token, 0) } == 0 {
        env = std::ptr::null_mut();
    }
    let mut cmd = wide(&format!("\"{}\" {args}", exe.display()).trim_end().to_owned());
    let app = wide(&exe.display().to_string());
    let dir = exe.parent().map(|d| wide(&d.display().to_string()));
    let mut desktop = wide("winsta0\\default");
    let result = spawn(
        |flags, pi| unsafe {
            let mut si: STARTUPINFOW = std::mem::zeroed();
            si.cb = size_of::<STARTUPINFOW>() as u32;
            si.lpDesktop = desktop.as_mut_ptr();
            si.dwFlags = STARTF_FORCEOFFFEEDBACK;
            CreateProcessAsUserW(
                token,
                app.as_ptr(),
                cmd.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                flags,
                env,
                dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
                &si,
                pi,
            ) != 0
        },
        CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP,
    );
    unsafe {
        if !env.is_null() {
            DestroyEnvironmentBlock(env);
        }
        CloseHandle(token);
    }
    result.map(drop).map_err(|e| format!("{session}. oturumda başlatılamadı (hata {e})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_runs_very_silent_without_reopening_ui() {
        let cmd = installer_command(Path::new(r"C:\ProgramData\GoodbyeDPI3\update\DPI-UI-Setup.exe"), Path::new(r"C:\ProgramData\GoodbyeDPI3\logs\update-setup.log"));
        assert!(cmd.starts_with("\"C:\\ProgramData\\GoodbyeDPI3\\update\\DPI-UI-Setup.exe\" "), "{cmd}");
        for flag in ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/UPDATE=service"] {
            assert!(cmd.split(' ').any(|a| a == flag), "{flag} eksik: {cmd}");
        }
        assert!(cmd.ends_with("/LOG=\"C:\\ProgramData\\GoodbyeDPI3\\logs\\update-setup.log\""), "{cmd}");
    }

    /// Servis disinda (SYSTEM degilken) oturum belirteci alinamaz; hata doner, cokmez.
    #[test]
    fn session_launch_fails_cleanly_without_tcb_privilege() {
        let exe = std::env::current_exe().unwrap();
        assert!(run_as_session_user(u32::MAX, &exe, "--version").is_err());
    }

    /// Sessiz baslatma yolu gercekten surec olusturur (ayrilmaya izin vermeyen is nesnesinde de).
    #[test]
    fn spawns_detached_process() {
        let dir = std::env::temp_dir().join(format!("gdpi3-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("calisti.txt");
        let exe = Path::new(r"C:\Windows\System32\cmd.exe");
        let mut cmd = wide(&format!("\"{}\" /c echo ok> \"{}\"", exe.display(), marker.display()));
        let app = wide(&exe.display().to_string());
        let process = spawn(
            |flags, pi| unsafe {
                let mut si: STARTUPINFOW = std::mem::zeroed();
                si.cb = size_of::<STARTUPINFOW>() as u32;
                CreateProcessW(app.as_ptr(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, flags, std::ptr::null(), std::ptr::null(), &si, pi) != 0
            },
            CREATE_NO_WINDOW,
        )
        .unwrap();
        assert!(process.wait(5000), "surec bitmedi");
        let mut ok = false;
        for _ in 0..50 {
            if std::fs::read_to_string(&marker).is_ok_and(|t| t.trim() == "ok") {
                ok = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(ok, "surec calismadi");
    }
}
