//! Hizmet gunlugu: son satirlar bellekte (arayuzdeki gunluk), hepsi `logs/service.log`
//! dosyasinda. Dosya 1 MB'i asinca `service.log.1` olur. Saatler UTC.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const RING: usize = 400;
const MAX_FILE: u64 = 1 << 20;

struct Log {
    ring: VecDeque<String>,
    file: Option<(PathBuf, File, u64)>,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("service.log");
    let file = OpenOptions::new().create(true).append(true).open(&path).ok().map(|f| {
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        (path, f, len)
    });
    *LOG.lock().unwrap() = Some(Log { ring: VecDeque::with_capacity(RING), file });
}

pub fn lines() -> Vec<String> {
    LOG.lock().unwrap().as_ref().map(|l| l.ring.iter().cloned().collect()).unwrap_or_default()
}

pub fn write(level: &str, msg: std::fmt::Arguments) {
    let line = format!("{} [{level}] {msg}", timestamp());
    let mut guard = LOG.lock().unwrap();
    let Some(log) = guard.as_mut() else {
        eprintln!("{line}");
        return;
    };
    if log.ring.len() == RING {
        log.ring.pop_front();
    }
    log.ring.push_back(line.clone());
    if let Some((path, f, len)) = log.file.as_mut() {
        if *len + line.len() as u64 + 1 > MAX_FILE {
            let rotated = path.with_extension("log.1");
            let _ = std::fs::remove_file(&rotated);
            let _ = std::fs::rename(&*path, &rotated);
            if let Ok(nf) = OpenOptions::new().create(true).append(true).open(&*path) {
                *f = nf;
                *len = 0;
            }
        }
        if writeln!(f, "{line}").is_ok() {
            *len += line.len() as u64 + 1;
        }
    }
}

/// "2026-10-24 08:31:05Z"
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Gun sayisindan takvim tarihi (Howard Hinnant, civil_from_days).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[macro_export]
macro_rules! info {
    ($($a:tt)*) => { $crate::log::write("bilgi", format_args!($($a)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($a:tt)*) => { $crate::log::write("uyarı", format_args!($($a)*)) };
}

#[macro_export]
macro_rules! error {
    ($($a:tt)*) => { $crate::log::write("hata", format_args!($($a)*)) };
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_shape() {
        let t = super::timestamp();
        assert_eq!(t.len(), 20, "{t}");
        assert!(t.starts_with("20") && t.ends_with('Z'));
    }
}
