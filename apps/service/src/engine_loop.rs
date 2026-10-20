//! WinDivert okuma/yazma dongusu.
//!
//! Kararlilik ilkeleri:
//! - Fail-open: dongu biterse ya da surec cokerse tutamac kapanir ve WinDivert
//!   yakaladigi trafigi normal akisina birakir. Internet asla kesilmez.
//! - Panik izolasyonu: tek paketin islenmesi panik yaparsa paket oldugu gibi
//!   gecirilir; dakikada cok fazla olursa motor durup sebebini bildirir.
//! - Toplu okuma (16 paket) ve paket basina heap tahsisi olmayan isleme.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use engine::{Event, IpPacket, ItemKind, Output, Processor, Verdict};
use shared::ipc::StatsSnapshot;
use windivert::{Address, Handle, Layer, OsError, Shutdown};

use crate::{error, warn};

const BATCH: usize = 16;
const BUF: usize = 256 * 1024;
const MAX_PANICS_PER_MINUTE: u32 = 50;

/// Dongunun denetleyiciye bildirdikleri.
pub enum EngineMsg {
    Learned { host: String, reason: engine::LearnReason },
    /// Dongu kendiliginden durdu (kurtarilamaz hata).
    Stopped { error: String },
    /// Yakalanan DNS sorgusu (sifreli DNS modunda).
    DnsQuery { packet: Vec<u8>, addr: Address },
}

pub struct RunningEngine {
    handle: Arc<Handle>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub stats: Arc<Mutex<StatsSnapshot>>,
    pub filter: String,
}

impl RunningEngine {
    /// Enjeksiyon icin tutamac (DNS yanitlari ayni tutamactan gonderilir).
    pub fn handle(&self) -> Arc<Handle> {
        self.handle.clone()
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Sira onemli: once Shutdown (bekleyen recv NO_DATA ile doner), is parcacigi
        // bitsin, en son tutamac dusurulsun (Arc'in son kopyasi kapatir).
        let _ = self.handle.shutdown(Shutdown::Both);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Motoru acar. Hata metni kullaniciya gosterilecek Turkce aciklamadir.
pub fn start(dir: &Path, mut processor: Processor, intercept_dns: bool, tx: Sender<EngineMsg>) -> Result<RunningEngine, String> {
    windivert::load(dir).map_err(|e| format!("WinDivert yüklenemedi: {e}. Kurulum klasörü eksik olabilir."))?;
    let filter = processor.build_filter();
    windivert::compile_filter(&filter, Layer::Network).map_err(|e| format!("Filtre oluşturulamadı: {e}"))?;

    let handle = Arc::new(Handle::open(&filter, Layer::Network, 0, 0).map_err(describe_open_error)?);
    // Paketler biz geri gonderene kadar bekler; kuyruk varsayilanlari (4096 paket, 2 sn) yeterli.

    let stop = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(Mutex::new(StatsSnapshot::default()));
    let thread = {
        let handle = handle.clone();
        let stop = stop.clone();
        let stats = stats.clone();
        std::thread::Builder::new()
            .name("motor".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                raise_priority();
                let result = run(&handle, &mut processor, intercept_dns, &stop, &stats, &tx);
                if let Err(e) = result {
                    if !stop.load(Ordering::SeqCst) {
                        error!("Motor durdu: {e}");
                        let _ = tx.send(EngineMsg::Stopped { error: e });
                    }
                }
            })
            .map_err(|e| format!("Motor iş parçacığı başlatılamadı: {e}"))?
    };
    Ok(RunningEngine { handle, stop, thread: Some(thread), stats, filter })
}

fn run(
    handle: &Handle,
    processor: &mut Processor,
    intercept_dns: bool,
    stop: &AtomicBool,
    stats: &Mutex<StatsSnapshot>,
    tx: &Sender<EngineMsg>,
) -> Result<(), String> {
    let mut buf = vec![0u8; BUF];
    let mut addrs = [Address::default(); BATCH];
    let mut out = Output::with_capacity(16 * 1024);
    let started = Instant::now();
    let mut consecutive_errors = 0u32;
    let mut panics = 0u32;
    let mut panic_window = Instant::now();
    let mut send_errors = 0u64;

    while !stop.load(Ordering::Relaxed) {
        let (len, count) = match handle.recv_batch(&mut buf, &mut addrs) {
            Ok(v) => {
                consecutive_errors = 0;
                v
            }
            Err(e) => {
                if stop.load(Ordering::Relaxed)
                    || matches!(e.0, OsError::NO_DATA | OsError::OPERATION_ABORTED | OsError::INVALID_HANDLE)
                {
                    return Ok(());
                }
                if e.0 == OsError::INSUFFICIENT_BUFFER {
                    continue;
                }
                consecutive_errors += 1;
                if consecutive_errors > 2000 {
                    return Err(format!("WinDivert okuma hatası sürüyor ({e})."));
                }
                if consecutive_errors > 16 {
                    std::thread::sleep(Duration::from_millis(10));
                }
                continue;
            }
        };

        let mut offset = 0;
        for addr in addrs.iter_mut().take(count) {
            let Some(plen) = packet_len(&buf[offset..len]) else { break };
            let pkt = &mut buf[offset..offset + plen];
            offset += plen;

            let now = started.elapsed().as_millis() as u64;
            let outbound = addr.outbound();
            let verdict = match catch_unwind(AssertUnwindSafe(|| processor.process(pkt, outbound, now, &mut out))) {
                Ok(v) => v,
                Err(_) => {
                    if panic_window.elapsed() > Duration::from_secs(60) {
                        panic_window = Instant::now();
                        panics = 0;
                    }
                    panics += 1;
                    if panics > MAX_PANICS_PER_MINUTE {
                        // Paketi yine de birak; sonra dur (tutamac kapaninca trafik normale doner).
                        let _ = handle.send(pkt, addr);
                        return Err("Motor beklenmedik paketlerde art arda hata verdi; güvenlik için durduruldu.".into());
                    }
                    warn!("Bir paket işlenirken hata oluştu; paket olduğu gibi geçirildi.");
                    out.clear();
                    Verdict::Pass
                }
            };

            let sent = match verdict {
                Verdict::Pass => handle.send(pkt, addr).is_ok(),
                Verdict::Drop => true,
                Verdict::Modified => {
                    windivert::calc_checksums(pkt, Some(addr), 0);
                    handle.send(pkt, addr).is_ok()
                }
                Verdict::DnsQuery => {
                    if intercept_dns {
                        // Orijinal sorgu tutulur; DNS isleyicisi yaniti enjekte eder ya da
                        // basarisiz olursa sorguyu oldugu gibi birakir.
                        let _ = tx.send(EngineMsg::DnsQuery { packet: pkt.to_vec(), addr: *addr });
                        true
                    } else {
                        handle.send(pkt, addr).is_ok()
                    }
                }
                Verdict::Replace => send_output(handle, &mut out, pkt, addr),
            };
            if !sent {
                send_errors += 1;
            }
        }

        // Olaylar ve sayaclar (paket grubu basina bir kez; kilit cekismesiz).
        for ev in processor.take_events() {
            let Event::Learned { host, reason } = ev;
            let _ = tx.send(EngineMsg::Learned { host, reason });
        }
        let s = processor.stats();
        if let Ok(mut snap) = stats.lock() {
            snap.bypassed = s.bypassed;
            snap.untouched = s.untouched;
            snap.fakes_sent = s.fakes_sent;
            snap.quic_dropped = s.quic_dropped;
            snap.voice_fakes_sent = s.voice_fakes_sent;
            snap.learned = s.learned;
            snap.engine_errors = send_errors + panics as u64;
        }
    }
    Ok(())
}

fn send_output(handle: &Handle, out: &mut Output, original: &[u8], addr: &Address) -> bool {
    let mut ok = true;
    for idx in 0..out.len() {
        let item = out.items()[idx];
        let mut a = *addr;
        match item.kind {
            ItemKind::Original => ok &= handle.send(original, &a).is_ok(),
            ItemKind::Built { recalc, corrupt_tcp_checksum } => {
                let bytes = out.bytes_mut(&item);
                if recalc {
                    windivert::calc_checksums(bytes, Some(&mut a), 0);
                    // Once dogru saglama, sonra bozma (GoodbyeDPI ile ayni): adres
                    // bayraklari gecerli kalir, NIC saglamayi yeniden hesaplamaz.
                    if corrupt_tcp_checksum {
                        if let Some(ip) = IpPacket::parse(bytes) {
                            ip.corrupt_tcp_checksum(bytes);
                        }
                    }
                }
                ok &= handle.send(bytes, &a).is_ok();
            }
        }
    }
    ok
}

/// Toplu okumada art arda dizilmis paketin uzunlugu (IP basligindan).
fn packet_len(b: &[u8]) -> Option<usize> {
    if b.is_empty() {
        return None;
    }
    let len = match b[0] >> 4 {
        4 if b.len() >= 20 => u16::from_be_bytes([b[2], b[3]]) as usize,
        6 if b.len() >= 40 => 40 + u16::from_be_bytes([b[4], b[5]]) as usize,
        _ => return None,
    };
    (len > 0 && len <= b.len()).then_some(len)
}

fn raise_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL};
    // Yakalanan her paket biz geri gonderene kadar bekler: gecikme olmasin.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) };
}

/// WinDivertOpen hatalarini kullanicinin anlayacagi dile cevirir.
pub fn describe_open_error(e: OsError) -> String {
    match e.0 {
        OsError::ACCESS_DENIED => "Erişim reddedildi. Motor yönetici yetkisiyle çalışmalı (servis kurulu mu?).".into(),
        OsError::FILE_NOT_FOUND => "WinDivert sürücüsü bulunamadı (WinDivert64.sys eksik olabilir).".into(),
        OsError::INVALID_IMAGE_HASH => {
            "WinDivert sürücüsü imza doğrulamasından geçemedi. Güvenlik yazılımı dosyayı değiştirmiş olabilir.".into()
        }
        OsError::DRIVER_BLOCKED => {
            "WinDivert sürücüsü engellendi. Antivirüs (özellikle Kaspersky) WinDivert'i engelliyor olabilir.".into()
        }
        OsError::DRIVER_FAILED_PRIOR_UNLOAD => {
            "Başka bir programın WinDivert sürücüsü kapanırken takıldı. Bilgisayarı yeniden başlatıp tekrar dene.".into()
        }
        OsError::SERVICE_DOES_NOT_EXIST => "WinDivert sürücü hizmeti kurulamadı.".into(),
        code => format!("WinDivert açılamadı (Windows hata kodu {code})."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_len_parses_v4_v6_and_rejects_garbage() {
        let mut v4 = vec![0u8; 60];
        v4[0] = 0x45;
        v4[2..4].copy_from_slice(&40u16.to_be_bytes());
        assert_eq!(packet_len(&v4), Some(40));
        let mut v6 = vec![0u8; 100];
        v6[0] = 0x60;
        v6[4..6].copy_from_slice(&20u16.to_be_bytes());
        assert_eq!(packet_len(&v6), Some(60));
        assert_eq!(packet_len(&[0x45, 0, 0xff, 0xff]), None);
        assert_eq!(packet_len(&[]), None);
    }
}
