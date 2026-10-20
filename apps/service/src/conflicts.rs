//! Ayni anda calisan baska WinDivert tabanli araclari bulur. Ayni paketleri iki
//! arac birden degistirince baglantilar bozulur; topluluk araclarinda en sik sorun bu.

use shared::ipc::Conflict;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

const KNOWN: &[(&str, &str)] = &[
    ("goodbyedpi.exe", "GoodbyeDPI (orijinal) çalışıyor"),
    ("goodbyedpi-ui.exe", "GoodbyeDPI UI (eski sürüm) çalışıyor"),
    ("winws.exe", "zapret (winws) çalışıyor"),
    ("winws2.exe", "zapret2 (winws2) çalışıyor"),
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
            if let Some((_, desc)) = KNOWN.iter().find(|(n, _)| *n == name) {
                found.push(Conflict { process: name, pid: entry.th32ProcessID, description: (*desc).to_owned() });
            }
            ok = Process32NextW(snap, &mut entry);
        }
        CloseHandle(snap);
    }
    found
}
