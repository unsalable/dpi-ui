//! Yakalanan DNS sorgularini sifreli DNS ile cozen is parcaciklari.
//!
//! Akis: sorgu (motor yakaladi, gonderilmedi) -> yerel ad / VPN / onbellek kontrolu
//! -> DoH -> yanit gelen paket olarak enjekte edilir. Her basarisizlikta guvenli
//! taraf: orijinal sorgu oldugu gibi gonderilir (sistem DNS'i), internet kesilmez.
//! Ust uste 3 hatada 30 sn boyunca dogrudan sistem DNS'ine birakilir.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engine::IpPacket;
use windivert::{Address, Handle};

use super::cache::{Cache, NEGATIVE_TTL};
use super::doh::DohClient;
use super::message;
use super::vpn::VpnDetector;

const WORKERS: usize = 2;
const QUEUE: usize = 256;
const CACHE_ENTRIES: usize = 1024;
const FAILS_BEFORE_DEGRADE: u32 = 3;
const DEGRADE_FOR: Duration = Duration::from_secs(30);

pub struct DnsJob {
    pub packet: Vec<u8>,
    pub addr: Address,
}

#[derive(Default)]
pub struct DnsStats {
    pub queries: AtomicU64,
    pub cached: AtomicU64,
    pub failures: AtomicU64,
    pub passed: AtomicU64,
}

/// Cozucu sagligi degisince denetleyiciye bildirilir.
pub type HealthCallback = Arc<dyn Fn(bool) + Send + Sync>;

pub struct Options {
    pub fallback_to_system: bool,
    pub skip_on_vpn: bool,
}

struct Health {
    fails: u32,
    degraded_until: Option<Instant>,
    healthy: bool,
}

struct Shared {
    handle: Arc<Handle>,
    client: DohClient,
    cache: Mutex<Cache>,
    health: Mutex<Health>,
    vpn: VpnDetector,
    stats: Arc<DnsStats>,
    opts: Options,
    on_health: HealthCallback,
}

pub struct Resolver {
    tx: SyncSender<DnsJob>,
    pub stats: Arc<DnsStats>,
}

impl Resolver {
    pub fn start(handle: Arc<Handle>, client: DohClient, opts: Options, on_health: HealthCallback) -> Self {
        let (tx, rx) = sync_channel::<DnsJob>(QUEUE);
        let stats = Arc::new(DnsStats::default());
        let shared = Arc::new(Shared {
            handle,
            client,
            cache: Mutex::new(Cache::new(CACHE_ENTRIES)),
            health: Mutex::new(Health { fails: 0, degraded_until: None, healthy: true }),
            vpn: VpnDetector::default(),
            stats: stats.clone(),
            opts,
            on_health,
        });
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..WORKERS {
            let shared = shared.clone();
            let rx = rx.clone();
            let _ = std::thread::Builder::new().name(format!("dns-{i}")).stack_size(256 * 1024).spawn(move || worker(&shared, &rx));
        }
        Self { tx, stats }
    }

    /// Sorguyu kuyruga koyar. Kuyruk doluysa sorgu cagiranca oldugu gibi gonderilmeli.
    pub fn submit(&self, job: DnsJob) -> Result<(), DnsJob> {
        match self.tx.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(j) | TrySendError::Disconnected(j)) => Err(j),
        }
    }
}

fn worker(s: &Shared, rx: &Mutex<Receiver<DnsJob>>) {
    loop {
        let job = {
            let Ok(guard) = rx.lock() else { return };
            match guard.recv() {
                Ok(j) => j,
                Err(_) => return, // motor durdu
            }
        };
        handle_job(s, job);
    }
}

fn pass_through(s: &Shared, job: &DnsJob) {
    s.stats.passed.fetch_add(1, Ordering::Relaxed);
    let _ = s.handle.send(&job.packet, &job.addr);
}

fn handle_job(s: &Shared, job: DnsJob) {
    let Some(ip) = IpPacket::parse(&job.packet) else { return pass_through(s, &job) };
    let payload = ip.payload(&job.packet);
    let Some(q) = message::parse_query(payload) else { return pass_through(s, &job) };
    if message::is_local_name(&q.name) || (s.opts.skip_on_vpn && s.vpn.is_vpn(job.addr.if_idx())) {
        return pass_through(s, &job);
    }
    s.stats.queries.fetch_add(1, Ordering::Relaxed);

    let now = Instant::now();
    if let Some(mut resp) = s.cache.lock().unwrap().get(&q.name, q.qtype, q.qclass, now) {
        s.stats.cached.fetch_add(1, Ordering::Relaxed);
        message::set_id(&mut resp, q.id);
        return inject(s, &job, &ip, payload, &resp, q.udp_size);
    }

    if let Some(until) = s.health.lock().unwrap().degraded_until {
        if now < until {
            return pass_through(s, &job);
        }
    }

    // DoH'ta onbellek dostu olsun diye kimlik 0 gonderilir (RFC 8484 4.1).
    let mut upstream = payload.to_vec();
    message::set_id(&mut upstream, 0);
    match s.client.resolve(&upstream) {
        Ok(mut resp) if resp.len() >= 12 => {
            record_health(s, true);
            let ttl = match message::rcode(&resp) {
                Some(0) => message::min_answer_ttl(&resp).unwrap_or(NEGATIVE_TTL),
                Some(3) => NEGATIVE_TTL,
                _ => 0,
            };
            if ttl > 0 {
                s.cache.lock().unwrap().put(&q.name, q.qtype, q.qclass, &resp, ttl, now);
            }
            message::set_id(&mut resp, q.id);
            inject(s, &job, &ip, payload, &resp, q.udp_size);
        }
        Ok(_) | Err(_) => {
            s.stats.failures.fetch_add(1, Ordering::Relaxed);
            record_health(s, false);
            if s.opts.fallback_to_system {
                pass_through(s, &job);
            }
            // Geri dusme kapaliysa sorgu yanitsiz kalir; istemci yeniden dener.
        }
    }
}

fn record_health(s: &Shared, ok: bool) {
    let mut h = s.health.lock().unwrap();
    let was = h.healthy;
    if ok {
        h.fails = 0;
        h.degraded_until = None;
        h.healthy = true;
    } else {
        h.fails += 1;
        if h.fails >= FAILS_BEFORE_DEGRADE {
            h.degraded_until = Some(Instant::now() + DEGRADE_FOR);
            h.healthy = false;
            h.fails = 0;
            s.client.reset();
        }
    }
    let now = h.healthy;
    drop(h);
    if was != now {
        (s.on_health)(now);
    }
}

/// Yaniti, sorgunun adres/portlari ters cevrilmis gelen UDP paketi olarak enjekte eder.
fn inject(s: &Shared, job: &DnsJob, ip: &IpPacket, query: &[u8], resp: &[u8], udp_size: u16) {
    let truncated;
    let resp = if resp.len() > udp_size as usize {
        match message::truncated_reply(query, resp) {
            Some(t) => {
                truncated = t;
                &truncated[..]
            }
            None => return pass_through(s, job),
        }
    } else {
        resp
    };
    let mut pkt = build_reply_packet(&job.packet, ip, resp);
    let mut addr = job.addr;
    addr.set_outbound(false);
    windivert::calc_checksums(&mut pkt, Some(&mut addr), 0);
    if s.handle.send(&pkt, &addr).is_err() {
        // Enjeksiyon olmadiysa sorguyu kaybetme.
        pass_through(s, job);
    }
}

pub fn build_reply_packet(query_pkt: &[u8], ip: &IpPacket, dns: &[u8]) -> Vec<u8> {
    let udp_len = 8 + dns.len();
    let mut p;
    if ip.v6 {
        p = vec![0u8; 40 + udp_len];
        p[0] = 0x60;
        p[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
        p[6] = 17;
        p[7] = 64;
        p[8..24].copy_from_slice(ip.dst_addr(query_pkt));
        p[24..40].copy_from_slice(ip.src_addr(query_pkt));
    } else {
        p = vec![0u8; 20 + udp_len];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&((20 + udp_len) as u16).to_be_bytes());
        p[6] = 0x40;
        p[8] = 64;
        p[9] = 17;
        p[12..16].copy_from_slice(ip.dst_addr(query_pkt));
        p[16..20].copy_from_slice(ip.src_addr(query_pkt));
    }
    let l4 = if ip.v6 { 40 } else { 20 };
    p[l4..l4 + 2].copy_from_slice(&ip.dst_port.to_be_bytes());
    p[l4 + 2..l4 + 4].copy_from_slice(&ip.src_port.to_be_bytes());
    p[l4 + 4..l4 + 6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    p[l4 + 8..].copy_from_slice(dns);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_packet_swaps_endpoints() {
        let q = message::tests::query("discord.com", 1, None);
        for v6 in [false, true] {
            let iph = if v6 { 40 } else { 20 };
            let mut pkt = vec![0u8; iph + 8 + q.len()];
            if v6 {
                pkt[0] = 0x60;
                pkt[4..6].copy_from_slice(&((8 + q.len()) as u16).to_be_bytes());
                pkt[6] = 17;
                pkt[8..24].copy_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7]);
                pkt[24..40].copy_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
            } else {
                pkt[0] = 0x45;
                pkt[2..4].copy_from_slice(&((iph + 8 + q.len()) as u16).to_be_bytes());
                pkt[9] = 17;
                pkt[12..16].copy_from_slice(&[192, 168, 1, 7]);
                pkt[16..20].copy_from_slice(&[192, 168, 1, 1]);
            }
            pkt[iph..iph + 2].copy_from_slice(&50123u16.to_be_bytes());
            pkt[iph + 2..iph + 4].copy_from_slice(&53u16.to_be_bytes());
            pkt[iph + 4..iph + 6].copy_from_slice(&((8 + q.len()) as u16).to_be_bytes());
            pkt[iph + 8..].copy_from_slice(&q);

            let ip = IpPacket::parse(&pkt).unwrap();
            let resp = message::tests::answer(&q, &[60]);
            let mut r = build_reply_packet(&pkt, &ip, &resp);
            engine::packet::fill_checksums(&mut r);
            let rip = IpPacket::parse(&r).unwrap();
            assert_eq!((rip.src_port, rip.dst_port), (53, 50123));
            assert_eq!(rip.src_addr(&r), ip.dst_addr(&pkt));
            assert_eq!(rip.dst_addr(&r), ip.src_addr(&pkt));
            assert_eq!(rip.payload(&r), &resp[..]);
            assert!(engine::packet::l4_checksum_valid(&r));
        }
    }
}
