//! Siteye giden ilk mesajin (TLS ClientHello ya da HTTP istegi) nasil parcalanip gidecegi.
//!
//! Windows motoru paketleri cekirdekte yakalayip degistirir. Proxy ise baglantinin bir ucu
//! oldugu icin yalnizca kendi yazdigini sekillendirebilir:
//! - **Bolme**: mesaj birkac TCP parcasi halinde gider (TCP_NODELAY: her yazma ayri parca).
//! - **Ters sira**: ilk parca TTL 1 ile gider ve ilk yonlendiricide duser; cekirdek onu biraz
//!   sonra normal TTL ile yeniden gonderir. Sunucuya da DPI'ya da once geri kalani ulasir.
//! - **TLS kaydi bolme**: ClientHello iki TLS kaydina ayrilir (RFC 8446 5.1 buna izin verir).
//! - **Host harfi**: HTTP isteginde "Host:" -> "hoSt:" (basliklar buyuk/kucuk harfe duyarsiz).

use std::ops::Range;

use engine::tls;
use serde::{Deserialize, Serialize};

/// Bolme konumu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pos {
    /// Mesajin basindan bu kadar bayt.
    Byte(u16),
    /// Ana bilgisayar adinin (TLS'te SNI, HTTP'de Host degeri) basindan bu kadar bayt sonra.
    Host(u16),
    /// Ana bilgisayar adinin ortasi.
    HostMid,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Desync {
    /// TCP parcalarinin sinirlari (konumlar ozgun mesaja gore).
    pub splits: Vec<Pos>,
    /// Ilk parca TTL 1 ile gitsin (ters sira).
    pub disorder: bool,
    /// ClientHello bu konumdan iki TLS kaydina bolunsun.
    pub tls_record: Option<Pos>,
    /// HTTP'de "Host:" basligi "hoSt:" yazilsin.
    pub host_case: bool,
}

impl Desync {
    pub fn is_noop(&self) -> bool {
        self.splits.is_empty() && self.tls_record.is_none() && !self.host_case
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tls,
    Http,
    Other,
}

pub fn kind(msg: &[u8]) -> Kind {
    if tls::is_client_hello(msg) {
        Kind::Tls
    } else if tls::http_host(msg).is_some() {
        Kind::Http
    } else {
        Kind::Other
    }
}

/// Ilk mesajin tamami kac bayt olmali? TLS kaydi parcali okunduysa proxy tamamlanmasini bekler.
pub fn wanted_len(msg: &[u8]) -> Option<usize> {
    if msg.len() >= 5 && msg[0] == 0x16 && msg[1] == 0x03 {
        let len = u16::from_be_bytes([msg[3], msg[4]]) as usize;
        return Some(5 + len.min(MAX_RECORD));
    }
    None
}

const MAX_RECORD: usize = 16 * 1024 + 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub range: Range<usize>,
    /// TTL 1 ile gonderilecek (ters sira).
    pub low_ttl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub data: Vec<u8>,
    pub segments: Vec<Segment>,
}

impl Plan {
    /// Mesaj tek parca, oldugu gibi mi gidiyor?
    pub fn is_plain(&self) -> bool {
        self.segments.len() == 1 && !self.segments[0].low_ttl
    }
}

pub fn plan(msg: &[u8], d: &Desync) -> Plan {
    let kind = kind(msg);
    if kind == Kind::Other || d.is_noop() {
        return Plan { data: msg.to_vec(), segments: vec![Segment { range: 0..msg.len(), low_ttl: false }] };
    }
    let host = host_range(msg, kind);
    let at = |p: Pos| -> Option<usize> {
        match p {
            Pos::Byte(n) => Some(n as usize),
            Pos::Host(n) => host.as_ref().map(|h| h.start + n as usize),
            Pos::HostMid => host.as_ref().map(|h| h.start + h.len() / 2),
        }
    };

    let mut data = msg.to_vec();
    // Kayit bolundugunde ikinci basligin konumu; sonrasindaki konumlar 5 bayt kayar.
    let mut record_cut = None;
    if kind == Kind::Tls {
        if let Some(p) = d.tls_record.and_then(at) {
            if let Some(split) = split_record(msg, p) {
                data = split;
                record_cut = Some(p);
            }
        }
    }
    if kind == Kind::Http && d.host_case {
        if let Some(i) = host_header_name(&data) {
            data[i..i + 4].copy_from_slice(b"hoSt");
        }
    }
    let shift = |p: usize| match record_cut {
        Some(cut) if p > cut => p + 5,
        _ => p,
    };

    let mut cuts: Vec<usize> = d.splits.iter().filter_map(|&p| at(p)).map(shift).filter(|&p| p > 0 && p < data.len()).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut segments = Vec::with_capacity(cuts.len() + 1);
    let mut start = 0;
    for end in cuts.into_iter().chain(std::iter::once(data.len())) {
        segments.push(Segment { range: start..end, low_ttl: false });
        start = end;
    }
    if d.disorder && segments.len() > 1 {
        segments[0].low_ttl = true;
    }
    Plan { data, segments }
}

/// SNI adinin (TLS) ya da Host degerinin (HTTP) mesajdaki yeri.
fn host_range(msg: &[u8], kind: Kind) -> Option<Range<usize>> {
    match kind {
        Kind::Tls => tls::find_sni(msg).map(|(off, len)| off..off + len),
        Kind::Http => {
            let host = tls::http_host(msg)?;
            // `http_host` mesajin icinden bir dilim dondurur.
            let off = host.as_ptr() as usize - msg.as_ptr() as usize;
            Some(off..off + host.len())
        }
        Kind::Other => None,
    }
}

/// Tek kayitlik ClientHello'yu `at` (mutlak konum) noktasindan iki kayda boler. Kayit
/// tamam degilse ya da konum kaydin icinde degilse `None`.
fn split_record(msg: &[u8], at: usize) -> Option<Vec<u8>> {
    if msg.len() < 5 || msg[0] != 0x16 {
        return None;
    }
    let len = u16::from_be_bytes([msg[3], msg[4]]) as usize;
    let end = 5 + len;
    if end > msg.len() || at <= 5 || at >= end {
        return None;
    }
    let first = at - 5;
    let mut out = Vec::with_capacity(msg.len() + 5);
    out.extend_from_slice(&msg[..3]);
    out.extend_from_slice(&(first as u16).to_be_bytes());
    out.extend_from_slice(&msg[5..at]);
    out.extend_from_slice(&msg[..3]);
    out.extend_from_slice(&((len - first) as u16).to_be_bytes());
    out.extend_from_slice(&msg[at..]);
    Some(out)
}

/// Basliklardaki "Host:" adinin ilk harfinin konumu.
fn host_header_name(msg: &[u8]) -> Option<usize> {
    let head_end = msg.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(msg.len());
    msg[..head_end].windows(7).position(|w| w[..2] == *b"\r\n" && w[2..].eq_ignore_ascii_case(b"host:")).map(|i| i + 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::fakes::build_client_hello;

    fn hello() -> Vec<u8> {
        build_client_hello("discord.com", 517, false)
    }

    fn sni(msg: &[u8]) -> Range<usize> {
        let (off, len) = tls::find_sni(msg).unwrap();
        off..off + len
    }

    /// Kayitlari birlestirip tek kayitlik ClientHello'ya geri cevirir (sunucunun gordugu).
    fn merge_records(data: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        let mut pos = 0;
        while pos + 5 <= data.len() {
            let len = u16::from_be_bytes([data[pos + 3], data[pos + 4]]) as usize;
            payload.extend_from_slice(&data[pos + 5..pos + 5 + len]);
            pos += 5 + len;
        }
        assert_eq!(pos, data.len(), "kayitlar tam");
        let mut out = data[..3].to_vec();
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        out.extend_from_slice(&payload);
        out
    }

    fn joined(p: &Plan) -> Vec<u8> {
        p.segments.iter().flat_map(|s| p.data[s.range.clone()].iter().copied()).collect()
    }

    #[test]
    fn disorder_splits_and_marks_only_the_first_part() {
        let msg = hello();
        let p = plan(&msg, &Desync { splits: vec![Pos::Byte(2)], disorder: true, ..Default::default() });
        assert_eq!(p.segments, vec![Segment { range: 0..2, low_ttl: true }, Segment { range: 2..msg.len(), low_ttl: false }]);
        assert_eq!(joined(&p), msg);
    }

    #[test]
    fn host_positions_follow_the_sni() {
        let msg = hello();
        let host = sni(&msg);
        let p = plan(&msg, &Desync { splits: vec![Pos::HostMid, Pos::Byte(1), Pos::Host(1)], ..Default::default() });
        let cuts: Vec<usize> = p.segments.iter().map(|s| s.range.end).collect();
        assert_eq!(cuts, vec![1, host.start + 1, host.start + host.len() / 2, msg.len()]);
        assert!(!p.segments.iter().any(|s| s.low_ttl));
    }

    #[test]
    fn record_split_keeps_the_handshake_and_moves_later_cuts() {
        let msg = hello();
        let host = sni(&msg);
        let mid = host.start + host.len() / 2;
        let p = plan(&msg, &Desync { tls_record: Some(Pos::HostMid), splits: vec![Pos::HostMid, Pos::Byte(2)], disorder: true, host_case: false });
        assert_eq!(p.data.len(), msg.len() + 5);
        assert_eq!(merge_records(&p.data), msg, "sunucu ayni ClientHello'yu okur");
        // Ikinci kaydin basligi tam bolme noktasinda; ayni konumdaki TCP bolmesi oraya duser.
        assert_eq!(&p.data[mid..mid + 3], &msg[..3]);
        let cuts: Vec<usize> = p.segments.iter().map(|s| s.range.end).collect();
        assert_eq!(cuts, vec![2, mid, p.data.len()]);
        assert!(p.segments[0].low_ttl);
        // Ilk kayitta adin yalnizca yarisi gorunur.
        assert!(!p.data[..mid].windows(11).any(|w| w == b"discord.com"));
    }

    #[test]
    fn record_split_needs_a_complete_record() {
        let msg = hello();
        let d = Desync { tls_record: Some(Pos::HostMid), ..Default::default() };
        let partial = &msg[..sni(&msg).end + 4];
        assert_eq!(plan(partial, &d).data, partial, "eksik kayit bolunmez");
        assert_eq!(plan(&msg, &Desync { tls_record: Some(Pos::Byte(3)), ..Default::default() }).data, msg, "baslik icine dusen konum");
    }

    #[test]
    fn http_host_case_and_split_in_host() {
        let msg = b"GET / HTTP/1.1\r\nHost: discord.com\r\nUser-Agent: x\r\n\r\n".to_vec();
        let p = plan(&msg, &Desync { splits: vec![Pos::HostMid], host_case: true, ..Default::default() });
        let text = String::from_utf8(p.data.clone()).unwrap();
        assert!(text.contains("\r\nhoSt: discord.com\r\n"), "{text}");
        assert_eq!(p.segments.len(), 2);
        assert_eq!(&p.data[p.segments[1].range.clone()][..5], b"rd.co");
    }

    #[test]
    fn unknown_protocols_and_noop_pass_untouched() {
        let ssh = b"SSH-2.0-OpenSSH_9.6\r\n".to_vec();
        let d = Desync { splits: vec![Pos::Byte(2)], disorder: true, ..Default::default() };
        assert!(plan(&ssh, &d).is_plain());
        assert!(plan(&hello(), &Desync::default()).is_plain());
        // Mesajdan uzun konumlar ve 0 yok sayilir.
        let p = plan(&hello(), &Desync { splits: vec![Pos::Byte(0), Pos::Byte(60000)], disorder: true, ..Default::default() });
        assert!(p.is_plain(), "tek parca ters siraya sokulmaz");
    }

    #[test]
    fn wanted_len_reads_the_record_header() {
        let msg = hello();
        assert_eq!(wanted_len(&msg[..10]), Some(msg.len()));
        assert_eq!(wanted_len(b"GET / HTTP/1.1\r\n"), None);
        assert_eq!(wanted_len(&[0x16, 3, 1]), None);
    }
}
