//! Ayni anda calisan baska WinDivert tabanli araclari bulur. Ayni paketleri iki
//! arac birden degistirince baglantilar bozulur; topluluk araclarinda en sik sorun bu.

use shared::ipc::Conflict;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

/// (surec, Turkce aciklama, Ingilizce aciklama)
const KNOWN: &[(&str, &str, &str)] = &[
    ("goodbyedpi.exe", "GoodbyeDPI (orijinal) çalışıyor", "GoodbyeDPI (original) is running"),
    ("goodbyedpi-ui.exe", "GoodbyeDPI UI (eski sürüm) çalışıyor", "GoodbyeDPI UI (old version) is running"),
    ("winws.exe", "zapret (winws) çalışıyor", "zapret (winws) is running"),
    ("winws2.exe", "zapret2 (winws2) çalışıyor", "zapret2 (winws2) is running"),
];

pub fn scan() -> Vec<Conflict> {
    let mut found = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return found;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry);
        while ok != 0 {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase();
            if let Some((_, tr, en)) = KNOWN.iter().find(|(n, _, _)| *n == name) {
                let description = if shared::i18n::english() { en } else { tr };
                found.push(Conflict { process: name, pid: entry.th32ProcessID, description: (*description).to_owned() });
            }
            ok = Process32NextW(snap, &mut entry);
        }
        CloseHandle(snap);
    }
    found
}
