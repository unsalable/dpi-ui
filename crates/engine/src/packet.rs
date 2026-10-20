//! IPv4/IPv6 + TCP/UDP paketlerinin hafif cozumlemesi.
//!
//! [`IpPacket`] yalnizca ofsetleri ve okunan alanlari tutar; tampona sahip olmaz.
//! Degistirme yardimcilari ayni yerlesime sahip herhangi bir tampona (orijinal ya
//! da kopyasi) yazar. Cozumleme basarisizsa paket dokunulmadan gecirilmelidir.
//! IPv6 uzanti basliklari ve IPv4 parcalari bilerek desteklenmez (TLS/DNS
//! trafigi bunlari kullanmaz; guvenli taraf: dokunma).

pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;

pub const TCP_FIN: u8 = 0x01;
pub const TCP_SYN: u8 = 0x02;
pub const TCP_RST: u8 = 0x04;
pub const TCP_ACK: u8 = 0x10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpPacket {
    pub v6: bool,
    pub protocol: u8,
    /// TCP/UDP basliginin baslangici.
    pub l4_offset: usize,
    pub payload_offset: usize,
    pub payload_len: usize,
    pub src_port: u16,
    pub dst_port: u16,
    pub tcp_seq: u32,
    pub tcp_ack: u32,
    pub tcp_flags: u8,
    /// IPv4 TTL / IPv6 Hop Limit.
    pub ttl: u8,
}

#[inline]
fn be16(p: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([p[at], p[at + 1]])
}

#[inline]
fn be32(p: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

impl IpPacket {
    pub fn parse(p: &[u8]) -> Option<Self> {
        if p.len() < 20 {
            return None;
        }
        match p[0] >> 4 {
            4 => Self::parse_v4(p),
            6 => Self::parse_v6(p),
            _ => None,
        }
    }

    fn parse_v4(p: &[u8]) -> Option<Self> {
        let ihl = ((p[0] & 0x0F) as usize) * 4;
        if ihl < 20 || ihl > p.len() {
            return None;
        }
        // IP parcasi (MF bayragi ya da sifir olmayan ofset): L4 basligi guvenilmez.
        if be16(p, 6) & 0x3FFF != 0 {
            return None;
        }
        Self::build(p, false, ihl, p[9])
    }

    fn parse_v6(p: &[u8]) -> Option<Self> {
        if p.len() < 40 {
            return None;
        }
        let next = p[6];
        if next != PROTO_TCP && next != PROTO_UDP {
            return None;
        }
        Self::build(p, true, 40, next)
    }

    fn build(p: &[u8], v6: bool, l4: usize, protocol: u8) -> Option<Self> {
        let payload_offset = match protocol {
            PROTO_TCP => {
                if l4 + 20 > p.len() {
                    return None;
                }
                let data_offset = ((p[l4 + 12] >> 4) as usize) * 4;
                if data_offset < 20 {
                    return None;
                }
                l4 + data_offset
            }
            PROTO_UDP => {
                if l4 + 8 > p.len() {
                    return None;
                }
                l4 + 8
            }
            _ => return None,
        };
        if payload_offset > p.len() {
            return None;
        }
        let tcp = protocol == PROTO_TCP;
        Some(Self {
            v6,
            protocol,
            l4_offset: l4,
            payload_offset,
            payload_len: p.len() - payload_offset,
            src_port: be16(p, l4),
            dst_port: be16(p, l4 + 2),
            tcp_seq: if tcp { be32(p, l4 + 4) } else { 0 },
            tcp_ack: if tcp { be32(p, l4 + 8) } else { 0 },
            tcp_flags: if tcp { p[l4 + 13] } else { 0 },
            ttl: if v6 { p[7] } else { p[8] },
        })
    }

    pub fn is_tcp(&self) -> bool {
        self.protocol == PROTO_TCP
    }

    pub fn is_udp(&self) -> bool {
        self.protocol == PROTO_UDP
    }

    pub fn is_syn_ack(&self) -> bool {
        self.is_tcp() && self.tcp_flags & (TCP_SYN | TCP_ACK) == (TCP_SYN | TCP_ACK)
    }

    pub fn is_rst(&self) -> bool {
        self.is_tcp() && self.tcp_flags & TCP_RST != 0
    }

    /// TCP/UDP basligi (secenekler dahil) uzunlugu.
    pub fn l4_header_len(&self) -> usize {
        self.payload_offset - self.l4_offset
    }

    pub fn addr_len(&self) -> usize {
        if self.v6 { 16 } else { 4 }
    }

    fn src_addr_offset(&self) -> usize {
        if self.v6 { 8 } else { 12 }
    }

    fn dst_addr_offset(&self) -> usize {
        if self.v6 { 24 } else { 16 }
    }

    pub fn payload<'a>(&self, p: &'a [u8]) -> &'a [u8] {
        &p[self.payload_offset..self.payload_offset + self.payload_len]
    }

    pub fn src_addr<'a>(&self, p: &'a [u8]) -> &'a [u8] {
        let o = self.src_addr_offset();
        &p[o..o + self.addr_len()]
    }

    pub fn dst_addr<'a>(&self, p: &'a [u8]) -> &'a [u8] {
        let o = self.dst_addr_offset();
        &p[o..o + self.addr_len()]
    }

    /// Adresi 128 bitlik anahtara cevirir (v4 icin ust bitler sifir).
    pub fn addr_key(&self, p: &[u8], source: bool) -> u128 {
        let a = if source { self.src_addr(p) } else { self.dst_addr(p) };
        let mut b = [0u8; 16];
        b[16 - a.len()..].copy_from_slice(a);
        u128::from_be_bytes(b)
    }

    // ------------------------------------------------------ degistirme

    pub fn set_ttl(&self, buf: &mut [u8], ttl: u8) {
        buf[if self.v6 { 7 } else { 8 }] = ttl;
    }

    pub fn set_tcp_seq(&self, buf: &mut [u8], seq: u32) {
        if self.is_tcp() {
            buf[self.l4_offset + 4..self.l4_offset + 8].copy_from_slice(&seq.to_be_bytes());
        }
    }

    pub fn set_tcp_ack(&self, buf: &mut [u8], ack: u32) {
        if self.is_tcp() {
            buf[self.l4_offset + 8..self.l4_offset + 12].copy_from_slice(&ack.to_be_bytes());
        }
    }

    /// TCP saglamasini bilerek bozar. Once dogru saglama hesaplanmis olmali.
    pub fn corrupt_tcp_checksum(&self, buf: &mut [u8]) {
        if self.is_tcp() {
            let at = self.l4_offset + 16;
            let v = be16(buf, at).wrapping_sub(1);
            buf[at..at + 2].copy_from_slice(&v.to_be_bytes());
        }
    }

    /// IPv4 toplam uzunlugunu / IPv6 payload uzunlugunu yazar.
    pub fn set_total_len(&self, buf: &mut [u8], total: usize) {
        if self.v6 {
            buf[4..6].copy_from_slice(&((total - 40) as u16).to_be_bytes());
        } else {
            buf[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        }
    }

    pub fn set_udp_len(&self, buf: &mut [u8], udp_len: usize) {
        if self.is_udp() {
            let at = self.l4_offset + 4;
            buf[at..at + 2].copy_from_slice(&(udp_len as u16).to_be_bytes());
        }
    }

    /// TCP veri ofsetini (bayt) yazar; bayrak bitleri korunur.
    pub fn set_tcp_header_len(&self, buf: &mut [u8], header_len: usize) {
        if self.is_tcp() {
            let at = self.l4_offset + 12;
            buf[at] = (((header_len / 4) as u8) << 4) | (buf[at] & 0x0F);
        }
    }

    pub fn set_src_addr(&self, buf: &mut [u8], addr: &[u8]) {
        let o = self.src_addr_offset();
        buf[o..o + self.addr_len()].copy_from_slice(addr);
    }

    pub fn set_dst_addr(&self, buf: &mut [u8], addr: &[u8]) {
        let o = self.dst_addr_offset();
        buf[o..o + self.addr_len()].copy_from_slice(addr);
    }

    pub fn set_src_port(&self, buf: &mut [u8], port: u16) {
        buf[self.l4_offset..self.l4_offset + 2].copy_from_slice(&port.to_be_bytes());
    }

    pub fn set_dst_port(&self, buf: &mut [u8], port: u16) {
        buf[self.l4_offset + 2..self.l4_offset + 4].copy_from_slice(&port.to_be_bytes());
    }
}

/// Akis anahtari: uzak adres + uzak port + yerel port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub remote: u128,
    pub remote_port: u16,
    pub local_port: u16,
}

impl FlowKey {
    pub fn outbound(ip: &IpPacket, p: &[u8]) -> Self {
        Self { remote: ip.addr_key(p, false), remote_port: ip.dst_port, local_port: ip.src_port }
    }

    pub fn inbound(ip: &IpPacket, p: &[u8]) -> Self {
        Self { remote: ip.addr_key(p, true), remote_port: ip.src_port, local_port: ip.dst_port }
    }
}

/// IP + L4 saglama toplamlarini hesaplar (WinDivert'ten bagimsiz; testlerde ve
/// surucuye gitmeden dogrulamada kullanilir). IPv4 basligi da yeniden hesaplanir.
pub fn fill_checksums(buf: &mut [u8]) {
    let Some(ip) = IpPacket::parse(buf) else { return };
    if !ip.v6 {
        let ihl = ip.l4_offset;
        buf[10] = 0;
        buf[11] = 0;
        let c = fold(sum_bytes(0, &buf[..ihl]));
        buf[10..12].copy_from_slice(&c.to_be_bytes());
    }
    let ck_at = ip.l4_offset + if ip.is_tcp() { 16 } else { 6 };
    buf[ck_at] = 0;
    buf[ck_at + 1] = 0;
    let mut c = fold(l4_sum(buf, &ip));
    if ip.is_udp() && c == 0 {
        c = 0xFFFF;
    }
    buf[ck_at..ck_at + 2].copy_from_slice(&c.to_be_bytes());
}

/// L4 saglamasi gecerli mi?
pub fn l4_checksum_valid(buf: &[u8]) -> bool {
    let Some(ip) = IpPacket::parse(buf) else { return false };
    fold(l4_sum(buf, &ip)) == 0
}

fn l4_sum(buf: &[u8], ip: &IpPacket) -> u32 {
    let l4_len = buf.len() - ip.l4_offset;
    let mut s = sum_bytes(0, ip.src_addr(buf));
    s = sum_bytes(s, ip.dst_addr(buf));
    s += ip.protocol as u32;
    s += l4_len as u32;
    sum_bytes(s, &buf[ip.l4_offset..])
}

fn sum_bytes(mut s: u32, b: &[u8]) -> u32 {
    let mut chunks = b.chunks_exact(2);
    for c in &mut chunks {
        s = s.wrapping_add(u16::from_be_bytes([c[0], c[1]]) as u32);
    }
    if let [last] = chunks.remainder() {
        s = s.wrapping_add((*last as u32) << 8);
    }
    s
}

fn fold(mut s: u32) -> u16 {
    while s >> 16 != 0 {
        s = (s & 0xFFFF) + (s >> 16);
    }
    !(s as u16)
}
