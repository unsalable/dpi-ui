#![allow(dead_code)]

use engine::{IpPacket, ItemKind, Output, Processor, Verdict};

pub const BLOCKED: &str = "discord.com";

#[derive(Clone, Debug)]
pub struct Sent {
    pub data: Vec<u8>,
    pub recalc: bool,
    pub corrupt: bool,
    pub original: bool,
}

impl Sent {
    pub fn ip(&self) -> IpPacket {
        IpPacket::parse(&self.data).expect("cikti paketi cozumlenemedi")
    }
    pub fn payload(&self) -> Vec<u8> {
        let ip = self.ip();
        ip.payload(&self.data).to_vec()
    }
}

/// Paketi isler ve ciktilari (orijinal dahil) kopyalar halinde dondurur.
pub fn run(proc: &mut Processor, pkt: &mut [u8], outbound: bool, now: u64) -> (Verdict, Vec<Sent>) {
    let mut out = Output::default();
    let v = proc.process(pkt, outbound, now, &mut out);
    let sent = out
        .items()
        .iter()
        .map(|it| match it.kind {
            ItemKind::Original => Sent { data: pkt.to_vec(), recalc: false, corrupt: false, original: true },
            ItemKind::Built { recalc, corrupt_tcp_checksum } => {
                Sent { data: out.bytes(it).to_vec(), recalc, corrupt: corrupt_tcp_checksum, original: false }
            }
        })
        .collect();
    (v, sent)
}

pub fn contains(data: &[u8], needle: &[u8]) -> bool {
    data.windows(needle.len()).any(|w| w == needle)
}

pub fn ip_len_matches(data: &[u8], v6: bool) -> bool {
    if v6 {
        u16::from_be_bytes([data[4], data[5]]) as usize == data.len() - 40
    } else {
        u16::from_be_bytes([data[2], data[3]]) as usize == data.len()
    }
}

/// TCP parcalarini SEQ'e gore dizip payload'u kurar; bosluk / ust uste binme varsa bos.
pub fn reassemble(segments: &[Sent], base_seq: u32) -> Vec<u8> {
    let mut parts: Vec<_> = segments.iter().map(|s| (s.ip(), s)).collect();
    parts.sort_by_key(|(ip, _)| ip.tcp_seq);
    let mut result = Vec::new();
    for (ip, s) in parts {
        if ip.tcp_seq.wrapping_sub(base_seq) as usize != result.len() {
            return Vec::new();
        }
        result.extend_from_slice(ip.payload(&s.data));
    }
    result
}

/// Linux sunucu gibi davranan alici: sira disini bekletir, ortusen baytlari atar.
pub fn receiver_view(sent: &[Sent], isn: u32) -> Vec<u8> {
    let mut stream: Vec<u8> = Vec::new();
    let mut queue: Vec<(i64, Vec<u8>)> = Vec::new();
    for s in sent {
        let ip = s.ip();
        let start = ip.tcp_seq.wrapping_sub(isn) as i32 as i64;
        queue.push((start, ip.payload(&s.data).to_vec()));
        loop {
            let mut progress = false;
            let mut i = 0;
            while i < queue.len() {
                let (st, ref bytes) = queue[i];
                if st + bytes.len() as i64 <= stream.len() as i64 {
                    queue.remove(i);
                    continue;
                }
                if st > stream.len() as i64 {
                    i += 1;
                    continue;
                }
                let skip = (stream.len() as i64 - st) as usize;
                let tail = bytes[skip..].to_vec();
                stream.extend_from_slice(&tail);
                queue.remove(i);
                progress = true;
                break;
            }
            if !progress {
                break;
            }
        }
    }
    stream
}

/// Her konum icin ILK gonderilen bayti tutan bir DPI'in gordugu akis.
pub fn first_wins_view(sent: &[Sent], isn: u32) -> Vec<u8> {
    let mut map = std::collections::HashMap::new();
    for s in sent {
        let ip = s.ip();
        let start = ip.tcp_seq.wrapping_sub(isn) as i32 as i64;
        for (k, b) in ip.payload(&s.data).iter().enumerate() {
            map.entry(start + k as i64).or_insert(*b);
        }
    }
    let mut out = Vec::new();
    let mut i = 0i64;
    while let Some(b) = map.get(&i) {
        out.push(*b);
        i += 1;
    }
    out
}

const LOCAL_V4: [u8; 4] = [192, 168, 0, 10];
const REMOTE_V4: [u8; 4] = [162, 159, 136, 232];
const LOCAL_V6: [u8; 16] = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10];
const REMOTE_V6: [u8; 16] = [0x26, 0x06, 0x47, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x68, 0x10];

#[derive(Default, Clone)]
pub struct Addr {
    pub swap: bool,
    pub dst_last: u8,
    pub src: Option<Vec<u8>>,
    pub dst: Option<Vec<u8>>,
}

fn write_ip(p: &mut [u8], v6: bool, protocol: u8, ttl: u8, a: &Addr) {
    let total = p.len();
    let local: Vec<u8> = if v6 { LOCAL_V6.to_vec() } else { LOCAL_V4.to_vec() };
    let mut remote: Vec<u8> = if v6 { REMOTE_V6.to_vec() } else { REMOTE_V4.to_vec() };
    if a.dst_last != 0 {
        remote = if v6 {
            let mut r = vec![0u8; 16];
            r[0] = 0xfe;
            r[1] = 0x80;
            r[15] = a.dst_last;
            r
        } else {
            vec![192, 168, 0, a.dst_last]
        };
    }
    let (mut src, mut dst) = if a.swap { (remote, local) } else { (local, remote) };
    if let Some(s) = &a.src {
        src = s.clone();
    }
    if let Some(d) = &a.dst {
        dst = d.clone();
    }
    if v6 {
        p[0] = 0x60;
        p[4..6].copy_from_slice(&((total - 40) as u16).to_be_bytes());
        p[6] = protocol;
        p[7] = ttl;
        p[8..24].copy_from_slice(&src);
        p[24..40].copy_from_slice(&dst);
    } else {
        p[0] = 0x45;
        p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        p[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
        p[6] = 0x40;
        p[8] = ttl;
        p[9] = protocol;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn tcp(v6: bool, sport: u16, dport: u16, seq: u32, ack: u32, flags: u8, ttl: u8, payload: &[u8], a: Addr) -> Vec<u8> {
    let iph = if v6 { 40 } else { 20 };
    let mut p = vec![0u8; iph + 20 + payload.len()];
    write_ip(&mut p, v6, 6, ttl, &a);
    p[iph..iph + 2].copy_from_slice(&sport.to_be_bytes());
    p[iph + 2..iph + 4].copy_from_slice(&dport.to_be_bytes());
    p[iph + 4..iph + 8].copy_from_slice(&seq.to_be_bytes());
    p[iph + 8..iph + 12].copy_from_slice(&ack.to_be_bytes());
    p[iph + 12] = 0x50;
    p[iph + 13] = flags;
    p[iph + 14..iph + 16].copy_from_slice(&65535u16.to_be_bytes());
    p[iph + 20..].copy_from_slice(payload);
    p
}

pub fn out_tcp(v6: bool, sport: u16, dport: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
    tcp(v6, sport, dport, seq, 42, 0x18, 128, payload, Addr::default())
}

pub fn syn_ack(v6: bool, local_port: u16, ttl: u8) -> Vec<u8> {
    tcp(v6, 443, local_port, 1, 1, 0x12, ttl, &[], Addr { swap: true, ..Addr::default() })
}

pub fn udp(v6: bool, sport: u16, dport: u16, payload: &[u8], a: Addr) -> Vec<u8> {
    let iph = if v6 { 40 } else { 20 };
    let mut p = vec![0u8; iph + 8 + payload.len()];
    write_ip(&mut p, v6, 17, 64, &a);
    p[iph..iph + 2].copy_from_slice(&sport.to_be_bytes());
    p[iph + 2..iph + 4].copy_from_slice(&dport.to_be_bytes());
    p[iph + 4..iph + 6].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    p[iph + 8..].copy_from_slice(payload);
    p
}

pub fn discord_discovery(ssrc: u32) -> Vec<u8> {
    let mut d = vec![0u8; 74];
    d[0..2].copy_from_slice(&1u16.to_be_bytes());
    d[2..4].copy_from_slice(&70u16.to_be_bytes());
    d[4..8].copy_from_slice(&ssrc.to_be_bytes());
    d
}

pub fn stun_binding(attr_bytes: usize) -> Vec<u8> {
    let mut d = vec![0u8; 20 + attr_bytes];
    d[0..2].copy_from_slice(&1u16.to_be_bytes());
    d[2..4].copy_from_slice(&(attr_bytes as u16).to_be_bytes());
    d[4..8].copy_from_slice(&0x2112A442u32.to_be_bytes());
    for (i, b) in d.iter_mut().enumerate().take(20).skip(8) {
        *b = (i * 11) as u8;
    }
    d
}
