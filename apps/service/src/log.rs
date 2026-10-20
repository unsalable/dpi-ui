//! Kucuk kayit tutucu: son satirlar bellekte (halka tampon), istenirse dosyaya da.
//! Dosya 1 MB'i asinca `service.log.1` olarak dondurulur; disk sinirsiz dolmaz.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const RING: usize = 400;
const MAX_FILE: u64 = 1 << 20;

pub struct Logger {
    ring: Mutex<VecDeque<String>>,
    file: Mutex<Option<(PathBuf, File, u64)>>,
    console: bool,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

pub fn init(console: bool, file: Option<PathBuf>) {
    let file = file.and_then(|path| {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let f = OpenOptions::new().create(true).append(true).open(&path).ok()?;
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        Some((path, f, len))
    });
    let _ = LOGGER.set(Logger { ring: Mutex::new(VecDeque::with_capacity(RING)), file: Mutex::new(file), console });
}

pub fn lines() -> Vec<String> {
    LOGGER.get().map(|l| l.ring.lock().unwrap().iter().cloned().collect()).unwrap_or_default()
}

pub fn write(level: &str, msg: std::fmt::Arguments) {
    let line = format!("{} [{level}] {msg}", timestamp());
    let Some(l) = LOGGER.get() else {
        eprintln!("{line}");
        return;
    };
    if l.console {
        println!("{line}");
    }
    {
        let mut ring = l.ring.lock().unwrap();
        if ring.len() == RING {
            ring.pop_front();
        }
        ring.push_back(line.clone());
    }
    let mut guard = l.file.lock().unwrap();
    if let Some((path, f, len)) = guard.as_mut() {
        let bytes = line.len() as u64 + 2;
        if *len + bytes > MAX_FILE {
            let rotated = path.with_extension("log.1");
            let _ = std::fs::remove_file(&rotated);
            let _ = std::fs::rename(&*path, &rotated);
            if let Ok(nf) = OpenOptions::new().create(true).append(true).open(&*path) {
                *f = nf;
                *len = 0;
            }
        }
        if writeln!(f, "{line}\r").is_ok() {
            *len += bytes;
        }
    }
}

fn timestamp() -> String {
    let t = unsafe {
        let mut st = std::mem::zeroed();
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st);
        st
    };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[macro_export]
macro_rules! info {
    ($($t:tt)*) => { $crate::log::write("BİLGİ", format_args!($($t)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($t:tt)*) => { $crate::log::write("UYARI", format_args!($($t)*)) };
}

#[macro_export]
macro_rules! error {
    ($($t:tt)*) => { $crate::log::write("HATA", format_args!($($t)*)) };
}
