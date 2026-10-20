//! Windows yardimcilari: tema, tek ornek, hizmeti baslatma, oturum acilisi,
//! kabuk komutlari ve kodla cizilen simgelerden HICON uretimi.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND};
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DeleteObject};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_SZ, RRF_RT_REG_DWORD, RegCloseKey, RegDeleteValueW,
    RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows_sys::Win32::System::Services::{CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_START, StartServiceW};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, OpenEventW, ResetEvent, SetEvent, EVENT_MODIFY_STATE};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO, SW_SHOWNORMAL};

use crate::iconart::{self, Mark};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Windows uygulama temasi acik mi? (Okunamazsa koyu varsayilir.)
pub fn system_prefers_light() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value: u32 = 0;
    let mut size = 4u32;
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut value as *mut _ as *mut c_void, &mut size)
    };
    rc == 0 && value == 1
}

/// Gorev cubugu ve sistem ogeleri acik temada mi? (Okunamazsa koyu varsayilir.)
pub fn system_uses_light_taskbar() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let mut value: u32 = 0;
    let mut size = 4u32;
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut value as *mut _ as *mut c_void, &mut size)
    };
    rc == 0 && value == 1
}

/// Windows'ta "Animasyon efektleri" acik mi?
pub fn animations_enabled() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
    let mut on: i32 = 1;
    unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut on as *mut _ as *mut c_void, 0) };
    on != 0
}

/// Ad alanina ek: testler (`GDPI3_TEST` ya da `GDPI3_INSTANCE`) kullanicinin acik
/// uygulamasiyla cakismasin diye ayri adlar kullanir.
pub fn instance_name(base: &str) -> String {
    let suffix = std::env::var("GDPI3_INSTANCE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var_os("GDPI3_TEST").map(|_| ".test".to_owned()))
        .unwrap_or_default();
    format!("{base}{suffix}")
}

const QUIT_EVENT: &str = "Local\\GoodbyeDPI3.Quit";

/// Pencere ve tepsi sureclerinin ortak "cik" olayi (elle sifirlanan: tetiklenince bekleyen
/// tum surecler uyanir). Acilista sifirlanir; onceki bir cikistan kalmis olabilir.
pub fn quit_event() -> HANDLE {
    let n = wide(&instance_name(QUIT_EVENT));
    unsafe {
        let h = CreateEventW(std::ptr::null(), 1, 0, n.as_ptr());
        if !h.is_null() {
            ResetEvent(h);
        }
        h
    }
}

/// Acik pencereyi ve tepsiyi kapatir (tepsideki "Cik", `--quit`).
pub fn signal_quit() {
    let n = wide(&instance_name(QUIT_EVENT));
    unsafe {
        let h = OpenEventW(EVENT_MODIFY_STATE, 0, n.as_ptr());
        if !h.is_null() {
            SetEvent(h);
            CloseHandle(h);
        }
    }
}

/// Adli olay ile tek ornek: ilk ornekse olayi (bekleme icin) dondurur; degilse
/// mevcut ornegin olayini tetikleyip `None` doner.
pub fn single_instance(name: &str) -> Option<HANDLE> {
    let n = wide(name);
    unsafe {
        let h = CreateEventW(std::ptr::null(), 0, 0, n.as_ptr());
        if h.is_null() {
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let other = OpenEventW(EVENT_MODIFY_STATE, 0, n.as_ptr());
            if !other.is_null() {
                SetEvent(other);
            }
            return None;
        }
        Some(h)
    }
}

/// Adli mutex ile tek ornek (tepsi). Sahip olunursa surec boyunca tutulur.
pub fn claim_mutex(name: &str) -> bool {
    let n = wide(name);
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 1, n.as_ptr());
        !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Motor hizmetini baslatir (kurulum, etkilesimli kullanicilara baslatma izni verir).
pub fn start_service() -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return Err("Hizmet yöneticisine bağlanılamadı.".into());
        }
        let svc = OpenServiceW(scm, wide(shared::SERVICE_NAME).as_ptr(), SERVICE_START);
        let result = if svc.is_null() {
            Err("Motor hizmeti kurulu değil. Uygulamayı yeniden kurmayı dene.".into())
        } else if StartServiceW(svc, 0, std::ptr::null()) == 0 && GetLastError() != 1056 {
            // 1056 = zaten calisiyor.
            Err(format!("Motor hizmeti başlatılamadı (hata {}).", GetLastError()))
        } else {
            Ok(())
        };
        if !svc.is_null() {
            CloseServiceHandle(svc);
        }
        CloseServiceHandle(scm);
        result
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "GoodbyeDPI3";

/// Oturum acilisinda tepsiyi baslat (HKCU\...\Run; yonetici gerekmez).
pub fn set_autorun(enabled: bool) -> bool {
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, KEY_SET_VALUE | KEY_READ, &mut key) != 0 {
            return false;
        }
        let ok = if enabled {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            let cmd = wide(&format!("\"{exe}\" --tray"));
            RegSetValueExW(key, wide(RUN_VALUE).as_ptr(), 0, REG_SZ, cmd.as_ptr().cast(), (cmd.len() * 2) as u32) == 0
        } else {
            let rc = RegDeleteValueW(key, wide(RUN_VALUE).as_ptr());
            rc == 0 || rc == 2
        };
        RegCloseKey(key);
        ok
    }
}

#[allow(dead_code)]
pub fn reg_dword_type() -> u32 {
    REG_DWORD
}

/// Dosya ya da adresi varsayilan uygulamayla acar.
pub fn shell_open(target: &str) {
    let t = wide(target);
    let verb = wide("open");
    unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), t.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
}

/// Kodla cizilen isaretten HICON (cagiran DestroyIcon ile birakir ya da surec boyunca tutar).
pub fn make_icon(size: u32, mark: Mark) -> HICON {
    let bgra = iconart::render_bgra(size, mark);
    unsafe {
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = CreateDIBSection(std::ptr::null_mut(), &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, std::ptr::null());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

/// Pencereye kodla cizilen simgeyi verir (gorev cubugu ve Alt+Tab).
pub fn set_window_icons(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, ICON_BIG, ICON_SMALL, SM_CXICON, SM_CXSMICON, SendMessageW, WM_SETICON};
    unsafe {
        let small = make_icon(GetSystemMetrics(SM_CXSMICON).max(16) as u32, Mark::App);
        let big = make_icon(GetSystemMetrics(SM_CXICON).max(32) as u32, Mark::App);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big as isize);
    }
}

/// Pencereyi katmanli yapip saydamligini ayarlar (kapanirken solma icin).
pub fn set_window_alpha(hwnd: HWND, alpha: u8) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW, WS_EX_LAYERED,
    };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex & WS_EX_LAYERED as isize == 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED as isize);
        }
        SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
    }
}

/// Sistem cercevesini temaya uydurur: koyu modda koyu golge ve ince koyu kenar cizgisi.
pub fn set_window_theme(hwnd: HWND, dark: bool) {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    let dark_mode: i32 = dark as i32;
    // COLORREF = 0x00BBGGRR: koyu #3A3A3C, acik #D1D1D6.
    let border: u32 = if dark { 0x003C_3A3A } else { 0x00D6_D1D1 };
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark_mode as *const _ as *const c_void, 4);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &border as *const _ as *const c_void, 4);
    }
}

/// Dosyanin SHA-256 ozeti (kucuk harf onaltilik), Windows CNG ile.
pub fn sha256_file(path: &std::path::Path) -> Option<String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_ALG_HANDLE, BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider, BCryptHash, BCryptOpenAlgorithmProvider,
    };
    let data = std::fs::read(path).ok()?;
    let mut alg: BCRYPT_ALG_HANDLE = std::ptr::null_mut();
    let mut out = [0u8; 32];
    unsafe {
        if BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_SHA256_ALGORITHM, std::ptr::null(), 0) != 0 {
            return None;
        }
        let rc = BCryptHash(alg, std::ptr::null(), 0, data.as_ptr(), data.len() as u32, out.as_mut_ptr(), 32);
        BCryptCloseAlgorithmProvider(alg, 0);
        if rc != 0 {
            return None;
        }
    }
    Some(shared::update::to_hex(&out))
}

/// Ana pencere sureci calisiyor mu? (Tek ornek olayi acik mi.)
pub fn window_running() -> bool {
    use windows_sys::Win32::System::Threading::SYNCHRONIZATION_SYNCHRONIZE;
    let n = wide(&instance_name("Local\\GoodbyeDPI3.Window"));
    unsafe {
        let h = OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, n.as_ptr());
        if h.is_null() {
            return false;
        }
        CloseHandle(h);
        true
    }
}

/// Servisin indirip dogruladigi kurulum paketini baslatir. Paket yalnizca SYSTEM'in yazabildigi
/// klasordedir; yine de baslatmadan once SHA-256'si servisin bildirdigiyle karsilastirilir.
/// Kurulum normal baslatilir, yonetici onayini (UAC) kendisi ister; boylece kurulum bitince
/// yeniden acilan uygulama yonetici olarak degil, kullanicinin kendi yetkisiyle calisir.
/// `reopen`: "window" (pencere de acilir) ya da "tray".
pub fn launch_update(expected_sha256: &str, reopen: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
    let path = shared::paths::update_installer();
    let actual = sha256_file(&path).ok_or("Güncelleme paketi bulunamadı.")?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err("Güncelleme paketi doğrulanamadı; kurulmadı.".into());
    }
    let file = wide(&path.display().to_string());
    let params = wide(&format!("/SILENT /SUPPRESSMSGBOXES /NORESTART /SP- /UPDATE={reopen}"));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOASYNC;
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_SHOWNORMAL;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(format!("Kurulum başlatılamadı (hata {}).", unsafe { GetLastError() }));
    }
    Ok(())
}
