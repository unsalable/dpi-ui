//! DNS mesajlarinin (RFC 1035) gereken kadar cozumlemesi: sorgu adi/turu, EDNS
//! UDP boyutu, yanit TTL'i. Her adimda sinir denetimi; tutarsizlikta `None`.

#[derive(Debug, PartialEq, Eq)]
pub struct Query {
    pub id: u16,
    /// Kucuk harf, sondaki nokta yok. Kok sorgusu icin bos.
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
    /// Istemcinin kabul ettigi en buyuk UDP yaniti (EDNS yoksa 512).
    pub udp_size: u16,
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// Standart tek soruluk sorgu mu? Degilse cagiran taraf dokunmadan gecirmeli.
pub fn parse_query(msg: &[u8]) -> Option<Query> {
    if msg.len() < 12 {
        return None;
    }
    let flags = be16(msg, 2)?;
    let qr = flags & 0x8000 != 0;
    let opcode = (flags >> 11) & 0x0F;
    if qr || opcode != 0 || be16(msg, 4)? != 1 || be16(msg, 6)? != 0 || be16(msg, 8)? != 0 {
        return None;
    }
    let (name, mut pos) = read_name(msg, 12)?;
    let qtype = be16(msg, pos)?;
    let qclass = be16(msg, pos + 2)?;
    pos += 4;

    let mut udp_size = 512u16;
    for _ in 0..be16(msg, 10)? {
        let (_, after) = read_name(msg, pos)?;
        let rtype = be16(msg, after)?;
        let class = be16(msg, after + 2)?;
        let rdlen = be16(msg, after + 8)? as usize;
        if rtype == 41 {
            udp_size = class.max(512);
        }
        pos = after + 10 + rdlen;
        if pos > msg.len() {
            return None;
        }
    }
    Some(Query { id: be16(msg, 0)?, name, qtype, qclass, udp_size })
}

/// Adi okur (sikistirma isaretcileri dahil). Doner: (ad, adin mesajdaki bitisi).
fn read_name(msg: &[u8], start: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut pos = start;
    let mut end = None;
    let mut jumps = 0;
    loop {
        let len = *msg.get(pos)? as usize;
        if len & 0xC0 == 0xC0 {
            let ptr = (be16(msg, pos)? & 0x3FFF) as usize;
            end.get_or_insert(pos + 2);
            jumps += 1;
            if jumps > 16 || ptr >= msg.len() {
                return None;
            }
            pos = ptr;
            continue;
        }
        if len & 0xC0 != 0 {
            return None;
        }
        if len == 0 {
            return Some((name, end.unwrap_or(pos + 1)));
        }
        let label = msg.get(pos + 1..pos + 1 + len)?;
        if !name.is_empty() {
            name.push('.');
        }
        for &b in label {
            name.push(b.to_ascii_lowercase() as char);
        }
        if name.len() > 255 {
            return None;
        }
        pos += 1 + len;
    }
}

/// Yanitin RCODE'u.
pub fn rcode(msg: &[u8]) -> Option<u8> {
    Some((be16(msg, 2)? & 0x000F) as u8)
}

/// Yanittaki cevap kayitlarinin en kucuk TTL'i; cevap yoksa `None` (olumsuz yanit).
pub fn min_answer_ttl(msg: &[u8]) -> Option<u32> {
    let qd = be16(msg, 4)?;
    let an = be16(msg, 6)?;
    if an == 0 {
        return None;
    }
    let mut pos = 12;
    for _ in 0..qd {
        let (_, after) = read_name(msg, pos)?;
        pos = after + 4;
    }
    let mut min: Option<u32> = None;
    for _ in 0..an {
        let (_, after) = read_name(msg, pos)?;
        let ttl = be32(msg, after + 4)?;
        let rdlen = be16(msg, after + 8)? as usize;
        min = Some(min.map_or(ttl, |m| m.min(ttl)));
        pos = after + 10 + rdlen;
        if pos > msg.len() {
            return None;
        }
    }
    min
}

/// Tek soruluk, ozyinelemeli (RD) sorgu; kimlik 0 (DoH icin onerilen).
pub fn build_query(name: &str, qtype: u16) -> Vec<u8> {
    let mut m = vec![0, 0, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.trim_end_matches('.').split('.').filter(|l| !l.is_empty()) {
        let label = &label.as_bytes()[..label.len().min(63)];
        m.push(label.len() as u8);
        m.extend_from_slice(label);
    }
    m.push(0);
    m.extend_from_slice(&qtype.to_be_bytes());
    m.extend_from_slice(&1u16.to_be_bytes());
    m
}

/// Yanittaki A ve AAAA adresleri (CNAME zinciri atlanir), yanittaki sirayla.
pub fn answer_addrs(msg: &[u8]) -> Vec<std::net::IpAddr> {
    let mut out = Vec::new();
    let (Some(qd), Some(an)) = (be16(msg, 4), be16(msg, 6)) else { return out };
    let mut pos = 12;
    for _ in 0..qd {
        let Some((_, after)) = read_name(msg, pos) else { return out };
        pos = after + 4;
    }
    for _ in 0..an {
        let Some((_, after)) = read_name(msg, pos) else { return out };
        let (Some(rtype), Some(class), Some(rdlen)) = (be16(msg, after), be16(msg, after + 2), be16(msg, after + 8)) else {
            return out;
        };
        let data = after + 10;
        let Some(rdata) = msg.get(data..data + rdlen as usize) else { return out };
        match (rtype, class, rdata.len()) {
            (1, 1, 4) => out.push(std::net::IpAddr::from(<[u8; 4]>::try_from(rdata).unwrap())),
            (28, 1, 16) => out.push(std::net::IpAddr::from(<[u8; 16]>::try_from(rdata).unwrap())),
            _ => {}
        }
        pos = data + rdlen as usize;
    }
    out
}

pub fn set_id(msg: &mut [u8], id: u16) {
    if msg.len() >= 2 {
        msg[..2].copy_from_slice(&id.to_be_bytes());
    }
}

/// Yanit cok buyukse: yalnizca soruyu iceren, TC bayrakli yanit (istemci TCP'ye gecer).
pub fn truncated_reply(query: &[u8], response: &[u8]) -> Option<Vec<u8>> {
    let (_, qend) = read_name(query, 12)?;
    let qend = qend + 4;
    let mut out = query.get(..qend)?.to_vec();
    let rflags = be16(response, 2)?;
    // QR=1, TC=1, RD istemciden, RA ve RCODE yanittan.
    let flags = 0x8000 | 0x0200 | (be16(query, 2)? & 0x0100) | (rflags & 0x0080) | (rflags & 0x000F);
    out[2..4].copy_from_slice(&flags.to_be_bytes());
    out[4..6].copy_from_slice(&1u16.to_be_bytes());
    out[6..12].fill(0);
    Some(out)
}

/// Yerel agda cozulmesi gereken adlar: yonlendirici/kurum DNS'ine birakilir.
pub fn is_local_name(name: &str) -> bool {
    const SUFFIXES: &[&str] = &[
        ".local",
        ".lan",
        ".home",
        ".home.arpa",
        ".internal",
        ".localdomain",
        ".intranet",
        ".corp",
        ".private",
        ".localhost",
    ];
    if name.is_empty() || !name.contains('.') {
        return true;
    }
    if SUFFIXES.iter().any(|s| name.ends_with(s) || name == &s[1..]) {
        return true;
    }
    // Ozel adreslerin ters sorgulari.
    if let Some(rev) = name.strip_suffix(".in-addr.arpa") {
        let octets: Vec<&str> = rev.split('.').rev().collect();
        return match octets.as_slice() {
            ["10", ..] | ["127", ..] | ["192", "168", ..] | ["169", "254", ..] => true,
            ["172", second, ..] => second.parse::<u8>().is_ok_and(|n| (16..=31).contains(&n)),
            _ => false,
        };
    }
    if let Some(rev) = name.strip_suffix(".ip6.arpa") {
        let nibbles: String = rev.split('.').rev().collect();
        return nibbles.starts_with("fe8")
            || nibbles.starts_with("fe9")
            || nibbles.starts_with("fea")
            || nibbles.starts_with("feb")
            || nibbles.starts_with("fc")
            || nibbles.starts_with("fd")
            || nibbles == "00000000000000000000000000000001";
    }
    false
}

#[cfg(any(test, feature = "test-helpers"))]
pub mod tests {
    use super::*;

    pub fn query(name: &str, qtype: u16, edns: Option<u16>) -> Vec<u8> {
        let mut m = vec![0xab, 0xcd, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, if edns.is_some() { 1 } else { 0 }];
        for label in name.split('.') {
            m.push(label.len() as u8);
            m.extend_from_slice(label.as_bytes());
        }
        m.push(0);
        m.extend_from_slice(&qtype.to_be_bytes());
        m.extend_from_slice(&1u16.to_be_bytes());
        if let Some(size) = edns {
            m.extend_from_slice(&[0, 0, 41]);
            m.extend_from_slice(&size.to_be_bytes());
            m.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        }
        m
    }

    /// Sorguya, ad isaretcisiyle iki A kaydi iceren yanit.
    pub fn answer(q: &[u8], ttls: &[u32]) -> Vec<u8> {
        let (_, qend) = read_name(q, 12).unwrap();
        let mut r = q[..qend + 4].to_vec();
        r[2] = 0x81;
        r[3] = 0x80;
        r[6..8].copy_from_slice(&(ttls.len() as u16).to_be_bytes());
        r[10..12].fill(0);
        for (i, ttl) in ttls.iter().enumerate() {
            r.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]);
            r.extend_from_slice(&ttl.to_be_bytes());
            r.extend_from_slice(&[0, 4, 104, 16, 0, i as u8]);
        }
        r
    }

    #[test]
    fn parses_query_and_edns() {
        let q = query("Discord.COM", 1, Some(1232));
        let p = parse_query(&q).unwrap();
        assert_eq!(p, Query { id: 0xabcd, name: "discord.com".into(), qtype: 1, qclass: 1, udp_size: 1232 });
        assert_eq!(parse_query(&query("example.com", 28, None)).unwrap().udp_size, 512);
        let mut resp = q.clone();
        resp[2] |= 0x80;
        assert!(parse_query(&resp).is_none(), "yanit sorgu degil");
        assert!(parse_query(&q[..14]).is_none());
    }

    #[test]
    fn reads_answer_ttls() {
        let q = query("discord.com", 1, None);
        let a = answer(&q, &[300, 60]);
        assert_eq!(min_answer_ttl(&a), Some(60));
        assert_eq!(rcode(&a), Some(0));
        assert_eq!(min_answer_ttl(&answer(&q, &[])), None);
    }

    #[test]
    fn built_query_parses_back() {
        let q = build_query("Discord.com.", 28);
        let p = parse_query(&q).unwrap();
        assert_eq!((p.id, p.name.as_str(), p.qtype, p.qclass), (0, "discord.com", 28, 1));
    }

    #[test]
    fn reads_answer_addresses_skipping_cname() {
        let q = query("discord.com", 1, None);
        let a = answer(&q, &[300, 60]);
        let ips: Vec<String> = answer_addrs(&a).iter().map(|i| i.to_string()).collect();
        assert_eq!(ips, ["104.16.0.0", "104.16.0.1"]);

        // CNAME (rdata: isaretci) + AAAA.
        let (_, qend) = read_name(&q, 12).unwrap();
        let mut r = q[..qend + 4].to_vec();
        r[2] = 0x81;
        r[6..8].copy_from_slice(&2u16.to_be_bytes());
        r.extend_from_slice(&[0xC0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 2, 0xC0, 12]);
        r.extend_from_slice(&[0xC0, 12, 0, 28, 0, 1, 0, 0, 0, 60, 0, 16]);
        r.extend_from_slice(&[0x26, 0x06, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11]);
        assert_eq!(answer_addrs(&r), ["2606:4700::1111".parse::<std::net::IpAddr>().unwrap()]);
        assert!(answer_addrs(&r[..r.len() - 3]).is_empty(), "kesik kayit atlanir");
    }

    #[test]
    fn truncation_keeps_question() {
        let q = query("big.example", 16, None);
        let a = answer(&q, &[60; 40]);
        let t = truncated_reply(&q, &a).unwrap();
        assert_eq!(&t[..2], &q[..2]);
        assert!(t[2] & 0x02 != 0, "TC bayragi");
        assert!(t[2] & 0x80 != 0, "QR bayragi");
        assert_eq!(parse_query(&q).unwrap().name, "big.example");
        assert_eq!(t.len(), q.len());
    }

    #[test]
    fn local_names() {
        for n in ["router", "nas.local", "printer.lan", "x.home.arpa", "1.0.168.192.in-addr.arpa", "5.20.172.in-addr.arpa", ""] {
            assert!(is_local_name(n), "{n}");
        }
        for n in ["discord.com", "8.8.8.8.in-addr.arpa", "5.40.172.in-addr.arpa", "local.example.com"] {
            assert!(!is_local_name(n), "{n}");
        }
    }

    #[test]
    fn rejects_pointer_loops() {
        let mut m = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        m.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]);
        assert!(parse_query(&m).is_none());
    }
}
