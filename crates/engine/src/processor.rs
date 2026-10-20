//! Motorun paket mantigi. WinDivert'e hic dokunmaz: gelen paket icin karar verir
//! ve gonderilecek paketleri [`Output`] icine kurar. Boylece surucu/yonetici
//! gerektirmeden uctan uca test edilebilir.
//!
//! GoodbyeDPI "-5 --set-ttl 5" davranisinin karsiligi (v1 ile ayni):
//!  1. ClientHello / HTTP isteginden once engelsiz siteye ait sahte istek
//!     (dusuk TTL ve/veya bozuk saglama / gecmis SEQ / MD5 ile).
//!  2. Gercek istek 2. bayttan (ve istege bagli SNI ortasindan) parcalara bolunur,
//!     parcalar ters sirada (istege bagli sira ortusmesiyle) gider.
//!  3. QUIC baslangic paketleri dusurulur; DNS yonlendirilir ya da yakalanir.
//!  4. Discord ses (IP Discovery) ve STUN paketlerinden once sahte UDP paketleri.
//!
//! v2'de yeni: kapsam (akilli liste + ogrenme), ek TLS portlari, sinirli bellek.

use std::collections::HashMap;
use std::hash::Hash;

use crate::config::{DpiConfig, FakePayload};
use crate::fakes;
use crate::output::{OutItem, Output};
use crate::packet::{FlowKey, IpPacket};
use crate::scope::{Decision, Scope};
use crate::tls;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Paketi oldugu gibi gecir.
    Pass,
    /// Paketi dusur.
    Drop,
    /// Paketin yerine `Output` icindekileri sirayla gonder.
    Replace,
    /// Paket yerinde degistirildi: saglamalari hesapla ve gonder.
    Modified,
    /// DNS sorgusu: yakalama modunda cagiran taraf cozer ve yaniti enjekte eder.
    DnsQuery,
}

/// DNS davranisi.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DnsMode {
    /// DNS'e dokunma.
    #[default]
    Off,
    /// Sorgulari secilen sunucunun (standart disi portuna) yonlendir (v1 yontemi).
    Redirect(Redirect),
    /// Sorgulari yakala ([`Verdict::DnsQuery`]); cagiran taraf sifreli DNS ile cozer.
    Intercept,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Redirect {
    pub v4: Option<([u8; 4], u16)>,
    pub v6: Option<([u8; 16], u16)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LearnReason {
    /// ClientHello yanitsiz kaldi ve yeniden gonderildi (sessiz engel).
    Retransmit,
    /// Sunucu tarafindan (ya da araya giren DPI'dan) RST geldi.
    Reset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Learned { host: String, reason: LearnReason },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub client_hellos: u64,
    pub hellos_without_sni: u64,
    pub hello_continuations: u64,
    pub continuation_splits: u64,
    pub overlap_skipped: u64,
    pub http_requests: u64,
    pub bypassed: u64,
    pub untouched: u64,
    pub fakes_sent: u64,
    pub quic_dropped: u64,
    pub voice_discoveries: u64,
    pub stun_messages: u64,
    pub voice_fakes_sent: u64,
    pub dns_redirected: u64,
    pub dns_answered: u64,
    pub dns_intercepted: u64,
    pub learned: u64,
}

// GoodbyeDPI --auto-ttl varsayilanlari (a1-a2-max) ve --min-ttl.
const AUTO_TTL1: i32 = 1;
const AUTO_TTL2: i32 = 4;
const AUTO_TTL_MAX: i32 = 10;
const MIN_HOPS: i32 = 3;

const TABLE_LIMIT: usize = 4096;
const ENTRY_LIFETIME_MS: u64 = 120_000;

/// Buyuk ClientHello'nun devam paketi olabilecek en buyuk TCP payload'u. Toplu
/// yukleme (MSS boyutunda) paketleri bunun ustunde kalir ve hic yakalanmaz.
pub const MAX_CONTINUATION_PAYLOAD: usize = 1200;

const PENDING_LIMIT: usize = 64;
const PENDING_LIFETIME_MS: u64 = 3_000;
/// Devam paketleri icin saklanan en fazla ClientHello verisi: SNI bunun icinde olmali.
const PENDING_CAP: usize = 4096;
const MAX_TLS_RECORD: usize = 16_384 + 5;

const WATCH_LIMIT: usize = 1024;
const WATCH_LIFETIME_MS: u64 = 10_000;
/// Windows'ta veri icin en kucuk yeniden gonderim suresi ~300 ms; bundan once
/// gelen ayni SEQ'li ClientHello yeniden gonderim sayilmaz.
const MIN_RETRANSMIT_MS: u64 = 200;
const EVENT_LIMIT: usize = 64;

/// TCP MD5 imzasi secenegi: NOP, NOP, tur 19, uzunluk 18, 16 bayt ozet.
pub const MD5_OPTION_LEN: usize = 20;

const DISCORD_DISCOVERY_HEAD: u32 = 0x0001_0046;
const STUN_MAGIC: u32 = 0x2112_A442;

#[derive(Clone, Copy)]
struct TtlRecord {
    ttl: u8,
    tick: u64,
}

struct PendingHello {
    next_seq: u32,
    data: Vec<u8>,
    len: usize,
    tick: u64,
}

struct Watch {
    host: Box<str>,
    seq: u32,
    tick: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DnsKey {
    v6: bool,
    client_port: u16,
    txid: u16,
}

#[derive(Clone, Copy)]
struct DnsMapping {
    addr: [u8; 16],
    port: u16,
    tick: u64,
}

pub struct Processor {
    cfg: DpiConfig,
    dns: DnsMode,
    scope: Scope,
    stats: Stats,
    ttl: HashMap<FlowKey, TtlRecord>,
    pending: HashMap<FlowKey, PendingHello>,
    watch: HashMap<FlowKey, Watch>,
    dns_map: HashMap<DnsKey, DnsMapping>,
    points: Vec<usize>,
    events: Vec<Event>,
}

#[inline]
fn be16(p: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([p[at], p[at + 1]])
}

#[inline]
fn be32(p: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

impl Processor {
    pub fn new(cfg: DpiConfig, dns: DnsMode, scope: Scope) -> Self {
        Self {
            cfg: cfg.sanitized(),
            dns,
            scope,
            stats: Stats::default(),
            ttl: HashMap::new(),
            pending: HashMap::new(),
            watch: HashMap::new(),
            dns_map: HashMap::new(),
            points: Vec::with_capacity(4),
            events: Vec::new(),
        }
    }

    pub fn config(&self) -> &DpiConfig {
        &self.cfg
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    pub fn scope_mut(&mut self) -> &mut Scope {
        &mut self.scope
    }

    /// Biriken olaylari (ogrenilen alan adlari) alir.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn uses_ttl_fake(&self) -> bool {
        self.cfg.fake_packet && (self.cfg.fake_ttl || !self.cfg.has_fake_protection())
    }

    fn tracks_continuation(&self) -> bool {
        self.cfg.split_tls && self.cfg.split_sni
    }

    fn uses_disorder_overlap(&self) -> bool {
        self.cfg.split_tls && self.cfg.reverse_split && self.cfg.seq_overlap > 0
    }

    fn tracks_server_ttl(&self) -> bool {
        (self.uses_ttl_fake() && self.cfg.auto_ttl) || self.uses_disorder_overlap()
    }

    fn is_tls_port(&self, port: u16) -> bool {
        port == 443 || self.cfg.extra_tls_ports.contains(&port)
    }

    // ================================================================ filtre

    /// Yalnizca gereken paketleri yakalayan WinDivert filtresi. Surekli veri akisi
    /// (indirme / MSS boyutunda yukleme) hic kullanici moduna cikmaz.
    pub fn build_filter(&self) -> String {
        const NO_LOCAL_V4: &str = "(ip.DstAddr < 127.0.0.1 or ip.DstAddr > 127.255.255.255) and \
            (ip.DstAddr < 10.0.0.0 or ip.DstAddr > 10.255.255.255) and \
            (ip.DstAddr < 192.168.0.0 or ip.DstAddr > 192.168.255.255) and \
            (ip.DstAddr < 172.16.0.0 or ip.DstAddr > 172.31.255.255) and \
            (ip.DstAddr < 169.254.0.0 or ip.DstAddr > 169.254.255.255)";
        const NO_LOCAL_V6: &str = "(ipv6.DstAddr > ::1) and \
            (ipv6.DstAddr < 2001::0 or ipv6.DstAddr > 2001:1::0) and \
            (ipv6.DstAddr < fc00::0 or ipv6.DstAddr > fe00::0) and \
            (ipv6.DstAddr < fe80::0 or ipv6.DstAddr > fec0::0) and \
            (ipv6.DstAddr < ff00::0 or ipv6.DstAddr > ffff::0)";

        let tls_dst = self.port_clause("tcp.DstPort");
        let tls_src = self.port_clause("tcp.SrcPort");
        let cfg = &self.cfg;
        let mut remote: Vec<String> = Vec::new();

        if cfg.fake_packet || cfg.split_tls {
            remote.push(format!(
                "(tcp and {tls_dst} and tcp.PayloadLength > 5 and \
                 tcp.Payload[0] == 0x16 and tcp.Payload[1] == 0x03 and tcp.Payload[5] == 0x01)"
            ));
        }
        if self.tracks_continuation() {
            remote.push(format!(
                "(tcp and {tls_dst} and tcp.PayloadLength > 0 and tcp.PayloadLength <= {MAX_CONTINUATION_PAYLOAD})"
            ));
        }
        if cfg.fragment_http && (cfg.fake_packet || cfg.split_tls) {
            remote.push(
                "(tcp and tcp.DstPort == 80 and tcp.PayloadLength > 4 and (\
                 tcp.Payload32[0] == 0x47455420 or tcp.Payload32[0] == 0x504F5354 or \
                 tcp.Payload32[0] == 0x48454144 or tcp.Payload32[0] == 0x50555420 or \
                 tcp.Payload32[0] == 0x4F505449 or tcp.Payload32[0] == 0x44454C45 or \
                 tcp.Payload32[0] == 0x50415443 or tcp.Payload32[0] == 0x434F4E4E))"
                    .to_owned(),
            );
        }
        if cfg.block_quic {
            remote.push("(udp and udp.DstPort == 443 and udp.PayloadLength >= 1200 and udp.Payload[0] >= 0xC0)".to_owned());
        }
        if cfg.voice_fake {
            remote.push(
                "(udp and udp.PayloadLength == 74 and udp.Payload32[0] == 0x00010046 and udp.Payload32[2] == 0)".to_owned(),
            );
            remote.push("(udp and udp.PayloadLength >= 20 and udp.Payload32[1] == 0x2112A442 and udp.Payload[0] < 0x40)".to_owned());
        }

        let mut clauses: Vec<String> = Vec::new();
        if !remote.is_empty() {
            clauses.push(format!(
                "(outbound and ((ip and {NO_LOCAL_V4}) or (ipv6 and {NO_LOCAL_V6})) and ({}))",
                remote.join(" or ")
            ));
        }
        if self.tracks_server_ttl() {
            clauses.push(format!("(inbound and tcp and tcp.Syn and tcp.Ack and ({tls_src} or tcp.SrcPort == 80))"));
        }
        if self.scope.observes_inbound() && (cfg.fake_packet || cfg.split_tls) {
            clauses.push(format!(
                "(inbound and tcp and {tls_src} and (tcp.Rst or (tcp.PayloadLength > 5 and \
                 tcp.Payload[0] == 0x16 and tcp.Payload[5] == 0x02)))"
            ));
        }
        match self.dns {
            DnsMode::Off => {}
            DnsMode::Intercept => {
                clauses.push("(outbound and udp and udp.DstPort == 53 and udp.PayloadLength >= 12)".to_owned());
            }
            DnsMode::Redirect(r) => {
                if let Some((addr, port)) = r.v4 {
                    let a = std::net::Ipv4Addr::from(addr);
                    clauses.push("(outbound and ip and udp and udp.DstPort == 53 and udp.PayloadLength >= 12)".to_owned());
                    clauses.push(format!("(inbound and ip and udp and udp.SrcPort == {port} and ip.SrcAddr == {a})"));
                }
                if let Some((addr, port)) = r.v6 {
                    let a = std::net::Ipv6Addr::from(addr);
                    clauses.push("(outbound and ipv6 and udp and udp.DstPort == 53 and udp.PayloadLength >= 12)".to_owned());
                    clauses.push(format!("(inbound and ipv6 and udp and udp.SrcPort == {port} and ipv6.SrcAddr == {a})"));
                }
            }
        }

        if clauses.is_empty() {
            return "false".to_owned();
        }
        format!("!impostor and !loopback and ({})", clauses.join(" or "))
    }

    fn port_clause(&self, field: &str) -> String {
        if self.cfg.extra_tls_ports.is_empty() {
            return format!("{field} == 443");
        }
        let mut s = format!("({field} == 443");
        for p in &self.cfg.extra_tls_ports {
            s.push_str(&format!(" or {field} == {p}"));
        }
        s.push(')');
        s
    }

    // ================================================================ karar

    /// Tek bir paketi degerlendirir. `Replace` donerse `out` icindekiler sirayla
    /// gonderilmeli; `Modified` donerse `p` yerinde degismistir.
    pub fn process(&mut self, p: &mut [u8], outbound: bool, now_ms: u64, out: &mut Output) -> Verdict {
        out.clear();
        let Some(ip) = IpPacket::parse(p) else { return Verdict::Pass };

        if ip.is_tcp() {
            if !outbound {
                if ip.is_syn_ack() {
                    self.record_server_ttl(&ip, p, now_ms);
                } else if self.scope.observes_inbound() {
                    self.observe_inbound(&ip, p);
                }
                return Verdict::Pass;
            }
            if self.is_tls_port(ip.dst_port) {
                let payload = ip.payload(p);
                if tls::is_client_hello(payload) {
                    return self.handle_client_hello(p, &ip, now_ms, out);
                }
                if ip.payload_len > 0 && !self.pending.is_empty() {
                    return self.handle_continuation(p, &ip, out, now_ms);
                }
                return Verdict::Pass;
            }
            if ip.dst_port == 80 && self.cfg.fragment_http && looks_like_http(ip.payload(p)) {
                let host = tls::http_host(ip.payload(p));
                return match self.scope.decide(host) {
                    Decision::Bypass => {
                        let changed = self.cfg.http_host_case && mix_host_header(p, &ip);
                        let v = self.handle_request(p, &ip, false, out, now_ms);
                        // Yalnizca baslik degistiyse paket yerinde degismis demektir.
                        if changed && v == Verdict::Pass { Verdict::Modified } else { v }
                    }
                    Decision::Pass | Decision::Watch(_) => {
                        self.stats.untouched += 1;
                        Verdict::Pass
                    }
                };
            }
            return Verdict::Pass;
        }

        if ip.is_udp() {
            if outbound {
                if ip.dst_port == 443 && self.cfg.block_quic && is_quic_initial(ip.payload(p)) {
                    self.stats.quic_dropped += 1;
                    return Verdict::Drop;
                }
                if ip.dst_port == 53 {
                    match self.dns {
                        DnsMode::Intercept if is_dns_query(ip.payload(p)) => {
                            self.stats.dns_intercepted += 1;
                            return Verdict::DnsQuery;
                        }
                        DnsMode::Redirect(r) => {
                            if self.redirect_dns_out(p, &ip, r, now_ms) {
                                self.stats.dns_redirected += 1;
                                return Verdict::Modified;
                            }
                        }
                        _ => {}
                    }
                    return Verdict::Pass;
                }
                if self.cfg.voice_fake {
                    let payload = ip.payload(p);
                    if is_discord_discovery(payload) {
                        self.stats.voice_discoveries += 1;
                        return self.handle_voice(p, &ip, out);
                    }
                    if is_stun_message(payload) {
                        self.stats.stun_messages += 1;
                        return self.handle_voice(p, &ip, out);
                    }
                }
            } else if let DnsMode::Redirect(r) = self.dns {
                if self.rewrite_dns_in(p, &ip, r) {
                    self.stats.dns_answered += 1;
                    return Verdict::Modified;
                }
            }
        }
        Verdict::Pass
    }

    // ================================================== TLS / HTTP istegi

    fn handle_client_hello(&mut self, p: &[u8], ip: &IpPacket, now_ms: u64, out: &mut Output) -> Verdict {
        let key = FlowKey::outbound(ip, p);
        let host = tls::sni_host(ip.payload(p));
        match self.scope.decide(host) {
            Decision::Bypass => self.handle_request(p, ip, true, out, now_ms),
            Decision::Pass => {
                self.stats.untouched += 1;
                Verdict::Pass
            }
            Decision::Watch(host) => {
                if let Some(w) = self.watch.get(&key) {
                    if w.seq == ip.tcp_seq && now_ms.saturating_sub(w.tick) >= MIN_RETRANSMIT_MS {
                        // Ayni ClientHello yeniden gonderiliyor: yanit gelmemis, sessiz engel.
                        // Bu tekrar paketine hemen atlatma uygula; site ilk denemede acilabilir.
                        let host = self.watch.remove(&key).map(|w| w.host).unwrap_or_default();
                        self.learn(&host, LearnReason::Retransmit);
                        return self.handle_request(p, ip, true, out, now_ms);
                    }
                    if w.seq == ip.tcp_seq {
                        self.stats.untouched += 1;
                        return Verdict::Pass;
                    }
                }
                if self.watch.len() >= WATCH_LIMIT {
                    prune(&mut self.watch, now_ms, |w| w.tick, WATCH_LIFETIME_MS, WATCH_LIMIT);
                }
                self.watch.insert(key, Watch { host: host.into(), seq: ip.tcp_seq, tick: now_ms });
                self.stats.untouched += 1;
                Verdict::Pass
            }
        }
    }

    fn observe_inbound(&mut self, ip: &IpPacket, p: &[u8]) {
        if !self.is_tls_port(ip.src_port) || self.watch.is_empty() {
            return;
        }
        let key = FlowKey::inbound(ip, p);
        if ip.is_rst() {
            if let Some(w) = self.watch.remove(&key) {
                self.learn(&w.host, LearnReason::Reset);
            }
        } else if tls::is_server_hello(ip.payload(p)) {
            self.watch.remove(&key);
        }
    }

    fn learn(&mut self, host: &str, reason: LearnReason) {
        if self.scope.learned.insert(host) {
            self.stats.learned += 1;
            if self.events.len() < EVENT_LIMIT {
                self.events.push(Event::Learned { host: host.to_ascii_lowercase(), reason });
            }
        }
    }

    fn handle_request(&mut self, p: &[u8], ip: &IpPacket, is_tls: bool, out: &mut Output, now_ms: u64) -> Verdict {
        self.stats.bypassed += 1;
        if is_tls {
            self.stats.client_hellos += 1;
            if tls::find_sni(ip.payload(p)).is_none() {
                self.stats.hellos_without_sni += 1;
            }
        } else {
            self.stats.http_requests += 1;
        }

        if self.cfg.fake_packet {
            let payload: &[u8] = match self.cfg.fake_payload {
                FakePayload::Zeros => &fakes::ZEROS,
                FakePayload::Tls if is_tls => fakes::tls_client_hello(),
                FakePayload::Tls => fakes::http_request(),
            };
            self.add_fakes(p, ip, payload, out, now_ms);
        }
        self.stats.fakes_sent += out.len() as u64;

        self.collect_split_points(p, ip, is_tls);
        if is_tls {
            self.track_continuation(p, ip, now_ms);
        }

        if !self.points.is_empty() {
            self.add_segments(p, ip, out, now_ms);
            return Verdict::Replace;
        }
        if out.is_empty() {
            return Verdict::Pass;
        }
        // Sahte paket gitti ama bolme yok: orijinal degismeden arkasindan.
        out.push_original();
        Verdict::Replace
    }

    // ------------------------------------------------------ sahte paket

    fn add_fakes(&mut self, p: &[u8], ip: &IpPacket, payload: &[u8], out: &mut Output, now_ms: u64) {
        let cfg = &self.cfg;
        // Koruma secilmemisse sahte istek sunucuya ulasip baglantiyi bozar: TTL'e dus.
        if cfg.fake_ttl || !cfg.has_fake_protection() {
            let ttl = self.resolve_fake_ttl(ip, p, now_ms);
            if ttl > 0 {
                self.add_fake(p, ip, payload, ttl, false, false, self.cfg.fake_md5sig, out);
            }
        }
        let cfg = &self.cfg;
        if cfg.fake_wrong_checksum || cfg.fake_wrong_seq || (cfg.fake_md5sig && !cfg.fake_ttl) {
            let (seq, cks, md5) = (cfg.fake_wrong_seq, cfg.fake_wrong_checksum, cfg.fake_md5sig);
            self.add_fake(p, ip, payload, 0, seq, cks, md5, out);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_fake(&self, p: &[u8], ip: &IpPacket, payload: &[u8], ttl: u8, wrong_seq: bool, corrupt: bool, md5: bool, out: &mut Output) {
        let split = self.cfg.split_fake && payload.len() > 2;
        let (first, second) = if split {
            let a = build_fake(out, p, ip, &payload[..2], 0, ttl, wrong_seq, corrupt, md5);
            let b = build_fake(out, p, ip, &payload[2..], 2, ttl, wrong_seq, corrupt, md5);
            (a, Some(b))
        } else {
            (build_fake(out, p, ip, payload, 0, ttl, wrong_seq, corrupt, md5), None)
        };
        // Ayni tampon tekrar gonderilebilir: saglama her gonderimde yeniden hesaplanir.
        for _ in 0..self.cfg.fake_repeats {
            out.push(first);
            if let Some(s) = second {
                out.push(s);
            }
        }
    }

    fn resolve_fake_ttl(&self, ip: &IpPacket, p: &[u8], now_ms: u64) -> u8 {
        let fixed = self.cfg.ttl.max(1);
        if !self.cfg.auto_ttl {
            return fixed;
        }
        match self.ttl.get(&FlowKey::outbound(ip, p)) {
            Some(rec) if now_ms.saturating_sub(rec.tick) < ENTRY_LIFETIME_MS => compute_auto_ttl(rec.ttl, fixed),
            _ => fixed,
        }
    }

    fn record_server_ttl(&mut self, ip: &IpPacket, p: &[u8], now_ms: u64) {
        if !self.tracks_server_ttl() {
            return;
        }
        if self.ttl.len() >= TABLE_LIMIT {
            prune(&mut self.ttl, now_ms, |r| r.tick, ENTRY_LIFETIME_MS, TABLE_LIMIT);
        }
        self.ttl.insert(FlowKey::inbound(ip, p), TtlRecord { ttl: ip.ttl, tick: now_ms });
    }

    /// Sunucu ortusen TCP verisinde ILK geleni mi tutuyor? Linux/BSD/macOS (baslangic
    /// TTL 64) son geleni tutar; Windows (128) ve ag cihazlari (255) ilk geleni. Ters
    /// sira ortusmesi bu sunucularda ClientHello'yu bozar (or. login.live.com).
    fn server_keeps_first_data(&self, ip: &IpPacket, p: &[u8], now_ms: u64) -> bool {
        self.ttl
            .get(&FlowKey::outbound(ip, p))
            .is_some_and(|r| now_ms.saturating_sub(r.tick) < ENTRY_LIFETIME_MS && is_first_wins_initial_ttl(r.ttl))
    }

    // ------------------------------------------------------- TCP bolme

    fn collect_split_points(&mut self, p: &[u8], ip: &IpPacket, is_tls: bool) {
        self.points.clear();
        if !self.cfg.split_tls {
            return;
        }
        let n = ip.payload_len;
        let pos = if self.cfg.split_position > 0 {
            self.cfg.split_position as usize
        } else if is_tls {
            0
        } else {
            2
        };
        if pos > 0 && pos < n {
            self.points.push(pos);
        }
        if is_tls && self.cfg.split_sni {
            if let Some((off, len)) = tls::find_sni(ip.payload(p)) {
                let mid = off + len / 2;
                if len >= 2 && mid > 0 && mid < n && !self.points.contains(&mid) {
                    self.points.push(mid);
                }
            }
        }
        self.points.sort_unstable();
    }

    fn add_segments(&mut self, p: &[u8], ip: &IpPacket, out: &mut Output, now_ms: u64) {
        let count = self.points.len() + 1;
        let overlap = self.cfg.seq_overlap as usize;

        // Sira ortusmesi: duz bolmede ilk parcaya, ters sirada ikinci parcaya (sondan bir
        // onceki gonderilen). Ters sirada ilk bolme konumundan kucuk olmali; yoksa sunucu
        // sahte baytlari ezemez. Ilk geleni tutan (Windows) sunucuda ters sira ortusmesi
        // istegi bozar: yalnizca ters sira bolme yapilir.
        let mut overlap_index = None;
        if overlap > 0 {
            if !self.cfg.reverse_split {
                overlap_index = Some(0);
            } else if count >= 2 && overlap < self.points[0] {
                if self.server_keeps_first_data(ip, p, now_ms) {
                    self.stats.overlap_skipped += 1;
                } else {
                    overlap_index = Some(1);
                }
            }
        }

        let mut segs = [OutItem { start: 0, len: 0, kind: crate::output::ItemKind::Original }; 4];
        let mut start = 0;
        for (i, seg) in segs.iter_mut().enumerate().take(count) {
            let end = if i < self.points.len() { self.points[i] } else { ip.payload_len };
            let len = end - start;
            // Paket orijinalinden buyumesin (MTU asimi olmasin).
            let ovl = if overlap_index == Some(i) && len + overlap <= ip.payload_len { overlap } else { 0 };
            *seg = build_segment(out, p, ip, start, len, ovl);
            start = end;
        }
        if self.cfg.reverse_split {
            segs[..count].reverse();
        }
        for s in &segs[..count] {
            out.push(*s);
        }
    }

    // ------------------------------------------- cok paketli ClientHello (ML-KEM)

    fn track_continuation(&mut self, p: &[u8], ip: &IpPacket, now_ms: u64) {
        if !self.tracks_continuation() {
            return;
        }
        let key = FlowKey::outbound(ip, p);
        let payload = ip.payload(p);
        let n = payload.len();
        let record_end = 5 + be16(payload, 3) as usize;
        let name_visible = tls::find_sni(payload).is_some_and(|(off, len)| off + len < n);

        if record_end <= n || record_end > MAX_TLS_RECORD || name_visible || n >= PENDING_CAP {
            self.pending.remove(&key);
            return;
        }
        if self.pending.len() >= PENDING_LIMIT {
            prune(&mut self.pending, now_ms, |h| h.tick, PENDING_LIFETIME_MS, PENDING_LIMIT);
        }
        let mut data = vec![0u8; record_end.min(PENDING_CAP)];
        data[..n].copy_from_slice(payload);
        self.pending.insert(key, PendingHello { next_seq: ip.tcp_seq.wrapping_add(n as u32), data, len: n, tick: now_ms });
    }

    fn handle_continuation(&mut self, p: &[u8], ip: &IpPacket, out: &mut Output, now_ms: u64) -> Verdict {
        let key = FlowKey::outbound(ip, p);
        let Some(hello) = self.pending.get_mut(&key) else { return Verdict::Pass };

        if ip.tcp_seq != hello.next_seq || now_ms.saturating_sub(hello.tick) > PENDING_LIFETIME_MS {
            // Siradaki paket degil (yeniden gonderim / baska veri) ya da cok eski.
            self.pending.remove(&key);
            return Verdict::Pass;
        }
        self.stats.hello_continuations += 1;

        let n = ip.payload_len;
        let head_len = hello.len;
        let take = n.min(hello.data.len() - head_len);
        hello.data[head_len..head_len + take].copy_from_slice(&ip.payload(p)[..take]);
        hello.len += take;
        hello.next_seq = hello.next_seq.wrapping_add(n as u32);

        let sni = tls::find_sni(&hello.data[..hello.len]);
        if hello.len >= hello.data.len() {
            self.pending.remove(&key);
        }
        let Some((name_off, name_len)) = sni else { return Verdict::Pass };
        if name_len < 2 {
            return Verdict::Pass;
        }
        // Ad bu pakette baslamiyorsa ve ortasi da burada degilse dokunma.
        let cut = (name_off + name_len / 2) as isize - head_len as isize;
        if cut <= 0 || cut as usize >= n {
            return Verdict::Pass;
        }
        let cut = cut as usize;
        self.stats.continuation_splits += 1;

        let a = build_segment(out, p, ip, 0, cut, 0);
        let b = build_segment(out, p, ip, cut, n - cut, 0);
        if self.cfg.reverse_split {
            out.push(b);
            out.push(a);
        } else {
            out.push(a);
            out.push(b);
        }
        Verdict::Replace
    }

    // ------------------------------------------------- Discord ses / STUN

    fn handle_voice(&mut self, p: &[u8], ip: &IpPacket, out: &mut Output) -> Verdict {
        let fake = build_udp_fake(out, p, ip, &fakes::UDP_ZEROS);
        for _ in 0..self.cfg.voice_fake_repeats {
            out.push(fake);
        }
        self.stats.voice_fakes_sent += self.cfg.voice_fake_repeats as u64;
        out.push_original();
        Verdict::Replace
    }

    // ------------------------------------------------------ DNS yonlendirme

    fn redirect_dns_out(&mut self, p: &mut [u8], ip: &IpPacket, r: Redirect, now_ms: u64) -> bool {
        let resolver: (&[u8], u16) = match (ip.v6, r.v4.as_ref(), r.v6.as_ref()) {
            (false, Some((a, port)), _) => (a, *port),
            (true, _, Some((a, port))) => (a, *port),
            _ => return false,
        };
        if !is_dns_query(ip.payload(p)) {
            return false;
        }
        if ip.dst_port == resolver.1 && ip.dst_addr(p) == resolver.0 {
            return false; // zaten dogru yere gidiyor
        }
        if self.dns_map.len() >= TABLE_LIMIT {
            prune(&mut self.dns_map, now_ms, |m| m.tick, ENTRY_LIFETIME_MS, TABLE_LIMIT);
        }
        let txid = be16(ip.payload(p), 0);
        let mut addr = [0u8; 16];
        addr[..ip.addr_len()].copy_from_slice(ip.dst_addr(p));
        self.dns_map
            .insert(DnsKey { v6: ip.v6, client_port: ip.src_port, txid }, DnsMapping { addr, port: ip.dst_port, tick: now_ms });
        ip.set_dst_addr(p, resolver.0);
        ip.set_dst_port(p, resolver.1);
        true
    }

    fn rewrite_dns_in(&mut self, p: &mut [u8], ip: &IpPacket, r: Redirect) -> bool {
        let resolver: (&[u8], u16) = match (ip.v6, r.v4.as_ref(), r.v6.as_ref()) {
            (false, Some((a, port)), _) => (a, *port),
            (true, _, Some((a, port))) => (a, *port),
            _ => return false,
        };
        let payload = ip.payload(p);
        if payload.len() < 12 || payload[2] & 0x80 == 0 {
            return false; // yanit degil
        }
        if ip.src_port != resolver.1 || ip.src_addr(p) != resolver.0 {
            return false;
        }
        // Kayit silinmez: ayni sorgunun tekrarina gelen ikinci yanit da geri yazilmali.
        let txid = be16(payload, 0);
        let Some(m) = self.dns_map.get(&DnsKey { v6: ip.v6, client_port: ip.dst_port, txid }).copied() else {
            return false;
        };
        ip.set_src_addr(p, &m.addr[..ip.addr_len()]);
        ip.set_src_port(p, m.port);
        true
    }
}

// ============================================================ yardimcilar

/// Orijinal IP+TCP basliklarini koruyup payload'u sahte icerikle degistirir.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_fake(
    out: &mut Output,
    p: &[u8],
    ip: &IpPacket,
    payload: &[u8],
    seq_offset: u32,
    ttl: u8,
    wrong_seq: bool,
    corrupt: bool,
    md5: bool,
) -> OutItem {
    let base = ip.payload_offset;
    let opt = if md5 && ip.l4_header_len() + MD5_OPTION_LEN <= 60 { MD5_OPTION_LEN } else { 0 };
    let start = out.begin();
    out.buf.extend_from_slice(&p[..base]);
    if opt > 0 {
        out.buf.extend_from_slice(&[0x01, 0x01, 19, 18]);
        out.buf.extend((0..16u32).map(|i| (0x5B ^ (i * 37)) as u8));
    }
    out.buf.extend_from_slice(payload);
    let total = out.buf.len() - start;

    let b = &mut out.buf[start..];
    if opt > 0 {
        ip.set_tcp_header_len(b, ip.l4_header_len() + opt);
    }
    ip.set_total_len(b, total);
    if ttl > 0 {
        ip.set_ttl(b, ttl);
    }
    let mut seq = ip.tcp_seq.wrapping_add(seq_offset);
    if wrong_seq {
        // GoodbyeDPI ile ayni kayma: sunucu pencere disi sayip atar, DPI yine isler.
        seq = seq.wrapping_sub(10_000);
        ip.set_tcp_ack(b, ip.tcp_ack.wrapping_sub(66_000));
    }
    ip.set_tcp_seq(b, seq);
    out.finish(start, true, corrupt)
}

/// Payload'un `[from, from+count)` dilimini iceren yeni bir IP+TCP paketi. SEQ dilim
/// ofseti kadar artar; `overlap` > 0 ise basa o kadar sifir eklenip SEQ geri cekilir.
pub(crate) fn build_segment(out: &mut Output, p: &[u8], ip: &IpPacket, from: usize, count: usize, overlap: usize) -> OutItem {
    let header = ip.payload_offset;
    let start = out.begin();
    out.buf.extend_from_slice(&p[..header]);
    out.buf.resize(out.buf.len() + overlap, 0);
    out.buf.extend_from_slice(&p[header + from..header + from + count]);
    let total = out.buf.len() - start;
    let b = &mut out.buf[start..];
    ip.set_total_len(b, total);
    ip.set_tcp_seq(b, ip.tcp_seq.wrapping_add(from as u32).wrapping_sub(overlap as u32));
    out.finish(start, true, false)
}

fn build_udp_fake(out: &mut Output, p: &[u8], ip: &IpPacket, payload: &[u8]) -> OutItem {
    let header = ip.payload_offset;
    let start = out.begin();
    out.buf.extend_from_slice(&p[..header]);
    out.buf.extend_from_slice(payload);
    let total = out.buf.len() - start;
    let b = &mut out.buf[start..];
    ip.set_total_len(b, total);
    ip.set_udp_len(b, total - ip.l4_offset);
    out.finish(start, true, false)
}

/// Sunucunun SYN-ACK TTL'inden hop sayisini tahmin edip sahte paketin TTL'ini
/// hesaplar (GoodbyeDPI tcp_get_auto_ttl). 0 = sunucu cok yakin, sahte gonderme.
/// Baslangic TTL'i taninmazsa `fallback` doner.
pub fn compute_auto_ttl(observed: u8, fallback: u8) -> u8 {
    let o = observed as i32;
    let hops = if (99..=128).contains(&o) {
        128 - o
    } else if (35..=64).contains(&o) {
        64 - o
    } else if (193..=255).contains(&o) {
        255 - o
    } else {
        return fallback;
    };
    if hops <= AUTO_TTL1 || hops < MIN_HOPS {
        return 0;
    }
    let mut ttl = hops - AUTO_TTL2;
    if ttl < AUTO_TTL2 && hops <= 9 {
        ttl = hops - AUTO_TTL1 - ((AUTO_TTL2 - AUTO_TTL1) as f64 * (hops as f64 / 10.0)) as i32;
    }
    ttl = ttl.min(AUTO_TTL_MAX);
    if ttl < 1 { 0 } else { ttl as u8 }
}

/// Gozlenen TTL 64'un ustundeyse sunucunun baslangic TTL'i 128 ya da 255'tir.
pub fn is_first_wins_initial_ttl(observed: u8) -> bool {
    observed > 64
}

/// `Host:` basligini (harf duzeni ne olursa olsun) `hoSt:` yapar.
pub fn mix_host_header(p: &mut [u8], ip: &IpPacket) -> bool {
    let start = ip.payload_offset;
    let end = start + ip.payload_len.min(2048);
    let Some(rel) = p[start..end].windows(7).position(|w| w[..2] == *b"\r\n" && w[2..6].eq_ignore_ascii_case(b"host") && w[6] == b':') else {
        return false;
    };
    let at = start + rel + 2;
    if p[at..at + 4] == *b"hoSt" {
        return false;
    }
    p[at..at + 4].copy_from_slice(b"hoSt");
    true
}

fn looks_like_http(payload: &[u8]) -> bool {
    if payload.len() < 5 {
        return false;
    }
    matches!(
        be32(payload, 0),
        0x4745_5420 | 0x504F_5354 | 0x4845_4144 | 0x5055_5420 | 0x4F50_5449 | 0x4445_4C45 | 0x5041_5443 | 0x434F_4E4E
    )
}

/// QUIC uzun baslikli istemci paketi (Initial, en az 1200 bayt, surum != 0).
fn is_quic_initial(payload: &[u8]) -> bool {
    payload.len() >= 1200 && payload[0] & 0xC0 == 0xC0 && be32(payload, 1) != 0
}

fn is_dns_query(payload: &[u8]) -> bool {
    payload.len() >= 12 && payload[2] & 0x80 == 0
}

/// Discord ses sunucusuna giden IP Discovery istegi: 74 bayt, tur 1, uzunluk 70,
/// SSRC ve ardindan sifirlarla dolu adres alani.
pub fn is_discord_discovery(payload: &[u8]) -> bool {
    payload.len() == 74 && be32(payload, 0) == DISCORD_DISCOVERY_HEAD && payload[8..72].iter().all(|&b| b == 0)
}

/// STUN mesaji (RFC 5389): ilk iki bit 0, sihirli cerez, uzunluk tutarli.
pub fn is_stun_message(payload: &[u8]) -> bool {
    if payload.len() < 20 || payload[0] >= 0x40 || be32(payload, 4) != STUN_MAGIC {
        return false;
    }
    let len = be16(payload, 2) as usize;
    len % 4 == 0 && len + 20 == payload.len()
}

/// Suresi dolanlari atar; hepsi tazeyse (asiri yuk) tabloyu sifirlar: bellek sinirsiz buyumez.
fn prune<K: Eq + Hash, V>(table: &mut HashMap<K, V>, now_ms: u64, tick: impl Fn(&V) -> u64, lifetime: u64, limit: usize) {
    table.retain(|_, v| now_ms.saturating_sub(tick(v)) <= lifetime);
    if table.len() >= limit {
        table.clear();
    }
}
