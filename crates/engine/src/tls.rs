//! TLS ClientHello icinde SNI (server_name) konumunu bulur. Her adimda sinir
//! denetimi yapilir; en ufak tutarsizlikta "bulunamadi" doner.

/// ClientHello mi? Kayit tipi 0x16, surum 0x03xx, handshake tipi 0x01.
pub fn is_client_hello(payload: &[u8]) -> bool {
    payload.len() >= 6 && payload[0] == 0x16 && payload[1] == 0x03 && payload[5] == 0x01
}

/// ServerHello mi? (Akilli listede "sunucu yanit verdi" isareti.)
pub fn is_server_hello(payload: &[u8]) -> bool {
    payload.len() >= 6 && payload[0] == 0x16 && payload[1] == 0x03 && payload[5] == 0x02
}

/// SNI adinin payload icindeki ofseti ve bu pakette gorunen uzunlugu.
pub fn find_sni(payload: &[u8]) -> Option<(usize, usize)> {
    let s = payload;
    if s.len() < 43 || s[0] != 0x16 {
        return None;
    }
    let mut pos = 5;
    if s[pos] != 0x01 {
        return None;
    }
    pos += 4 + 2 + 32;
    if pos + 1 > s.len() {
        return None;
    }
    pos += 1 + s[pos] as usize; // session_id
    if pos + 2 > s.len() {
        return None;
    }
    pos += 2 + u16::from_be_bytes([s[pos], s[pos + 1]]) as usize; // cipher_suites
    if pos + 1 > s.len() {
        return None;
    }
    pos += 1 + s[pos] as usize; // compression_methods
    if pos + 2 > s.len() {
        return None;
    }
    let ext_total = u16::from_be_bytes([s[pos], s[pos + 1]]) as usize;
    pos += 2;
    let ext_end = s.len().min(pos + ext_total);

    while pos + 4 <= ext_end {
        let ext_type = u16::from_be_bytes([s[pos], s[pos + 1]]);
        let ext_len = u16::from_be_bytes([s[pos + 2], s[pos + 3]]) as usize;
        let data = pos + 4;
        if ext_type == 0x0000 {
            if data + 5 > s.len() || s[data + 2] != 0 {
                return None;
            }
            let len = u16::from_be_bytes([s[data + 3], s[data + 4]]) as usize;
            let start = data + 5;
            if len == 0 || start >= s.len() {
                return None;
            }
            return Some((start, len.min(s.len() - start)));
        }
        pos = data + ext_len;
    }
    None
}

/// SNI ana bilgisayar adini (tamami bu pakette gorunuyorsa) kucuk harfle dondurur.
pub fn sni_host(payload: &[u8]) -> Option<&str> {
    let (off, len) = find_sni(payload)?;
    let declared = {
        // Bildirilen uzunluk, ofsetin hemen oncesindeki 2 bayt.
        u16::from_be_bytes([payload[off - 2], payload[off - 1]]) as usize
    };
    if len < declared {
        return None;
    }
    let name = std::str::from_utf8(&payload[off..off + len]).ok()?;
    valid_hostname(name).then_some(name)
}

/// HTTP istek basligindan Host degerini cikarir.
pub fn http_host(payload: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(&payload[..payload.len().min(2048)]).ok().or_else(|| {
        // Govde ikili olabilir; yalnizca basliklari dene.
        let end = payload.windows(4).position(|w| w == b"\r\n\r\n")?;
        std::str::from_utf8(&payload[..end]).ok()
    })?;
    for line in text.split("\r\n").skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("host") {
                let host = v.trim();
                let host = host.rsplit_once(':').map_or(host, |(h, port)| {
                    if port.bytes().all(|b| b.is_ascii_digit()) { h } else { host }
                });
                return valid_hostname(host).then_some(host);
            }
        }
    }
    None
}

fn valid_hostname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
}
