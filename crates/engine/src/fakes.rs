//! Sahte paket icerikleri.
//!
//! DPI once ENGELSIZ bir siteye (www.w3.org) giden gecerli gorunumlu bir istek
//! gorur, akisi o siteye ait sayar ve gercek istegi incelemeyi birakir. Sahte
//! paket asla gercek istegin kopyasi olmamali: kopya engelli SNI'yi DPI'a bir kez
//! daha gosterir (v1'deki Discord/Roblox hatasinin sebebi buydu).

use std::sync::OnceLock;

pub const FAKE_HOST: &str = "www.w3.org";

/// Bos sahte TCP icerigi (zapret2 blob=0x00000000).
pub const ZEROS: [u8; 4] = [0; 4];

/// Discord ses / STUN paketlerinden once giden sahte UDP icerigi (64 sifir).
pub const UDP_ZEROS: [u8; 64] = [0; 64];

pub fn http_request() -> &'static [u8] {
    static V: OnceLock<Vec<u8>> = OnceLock::new();
    V.get_or_init(|| {
        format!(
            "GET / HTTP/1.1\r\nHost: {FAKE_HOST}\r\nUser-Agent: curl/7.65.3\r\nAccept: */*\r\nAccept-Encoding: deflate, gzip, br\r\n\r\n"
        )
        .into_bytes()
    })
}

/// Sahte TLS ClientHello, SNI = www.w3.org, 517 bayt.
pub fn tls_client_hello() -> &'static [u8] {
    static V: OnceLock<Vec<u8>> = OnceLock::new();
    V.get_or_init(|| build_client_hello(FAKE_HOST, 517, false))
}

/// Tarayici ClientHello'suna benzeyen iyi bicimli bir TLS 1.2/1.3 ClientHello.
/// Rastgele alanlar sabittir. `sni_last` SNI uzantisini dolgudan sonra koyar
/// (Chromium'un buyuk ML-KEM isteginde ad ikinci TCP paketine dusebiliyor).
pub fn build_client_hello(host: &str, target_record_len: usize, sni_last: bool) -> Vec<u8> {
    let host = host.as_bytes();
    let mut ext = Vec::with_capacity(target_record_len);

    let mut sni = Vec::new();
    u16_to(&mut sni, host.len() + 3);
    sni.push(0x00);
    u16_to(&mut sni, host.len());
    sni.extend_from_slice(host);
    if !sni_last {
        extension(&mut ext, 0x0000, &sni);
    }

    extension(&mut ext, 0x0017, &[]);
    extension(&mut ext, 0xFF01, &[0x00]);
    extension(&mut ext, 0x000A, &[0x00, 0x06, 0x00, 0x1D, 0x00, 0x17, 0x00, 0x18]);
    extension(&mut ext, 0x000B, &[0x01, 0x00]);
    extension(&mut ext, 0x0023, &[]);
    extension(&mut ext, 0x0010, b"\x00\x0c\x02h2\x08http/1.1");
    extension(&mut ext, 0x0005, &[0x01, 0x00, 0x00, 0x00, 0x00]);
    extension(
        &mut ext,
        0x000D,
        &[0x00, 0x10, 0x04, 0x03, 0x08, 0x04, 0x04, 0x01, 0x05, 0x03, 0x08, 0x05, 0x05, 0x01, 0x08, 0x06, 0x06, 0x01],
    );
    extension(&mut ext, 0x002B, &[0x04, 0x03, 0x04, 0x03, 0x03]);
    extension(&mut ext, 0x002D, &[0x01, 0x01]);

    let mut key_share = Vec::new();
    u16_to(&mut key_share, 2 + 2 + 32);
    u16_to(&mut key_share, 0x001D);
    u16_to(&mut key_share, 32);
    key_share.extend(pattern(32, 0x5A));
    extension(&mut ext, 0x0033, &key_share);

    let mut body = vec![0x03, 0x03];
    body.extend(pattern(32, 0x3C));
    body.push(32);
    body.extend(pattern(32, 0x91));
    const SUITES: [u8; 30] = [
        0x13, 0x01, 0x13, 0x02, 0x13, 0x03, 0xC0, 0x2B, 0xC0, 0x2F, 0xC0, 0x2C, 0xC0, 0x30, 0xCC, 0xA9, 0xCC, 0xA8, 0xC0,
        0x13, 0xC0, 0x14, 0x00, 0x9C, 0x00, 0x9D, 0x00, 0x2F, 0x00, 0x35,
    ];
    u16_to(&mut body, SUITES.len());
    body.extend_from_slice(&SUITES);
    body.push(0x01);
    body.push(0x00);

    const FIXED_OVERHEAD: usize = 5 + 4 + 2 + 4;
    let sni_ext = if sni_last { 4 + sni.len() } else { 0 };
    let used = FIXED_OVERHEAD + body.len() + ext.len() + sni_ext;
    if target_record_len >= used {
        extension(&mut ext, 0x0015, &vec![0u8; target_record_len - used]);
    }
    if sni_last {
        extension(&mut ext, 0x0000, &sni);
    }

    u16_to(&mut body, ext.len());
    body.extend_from_slice(&ext);

    let mut hs = vec![0x01, (body.len() >> 16) as u8, (body.len() >> 8) as u8, body.len() as u8];
    hs.extend_from_slice(&body);

    let mut record = vec![0x16, 0x03, 0x01];
    u16_to(&mut record, hs.len());
    record.extend_from_slice(&hs);
    record
}

fn extension(dst: &mut Vec<u8>, ty: u16, data: &[u8]) {
    u16_to(dst, ty as usize);
    u16_to(dst, data.len());
    dst.extend_from_slice(data);
}

fn u16_to(dst: &mut Vec<u8>, v: usize) {
    dst.push((v >> 8) as u8);
    dst.push(v as u8);
}

/// v1 ile ayni deterministik dolgu: (seed * (i + 7)) ^ (i * 31), bayta kesilir.
fn pattern(count: usize, seed: u8) -> impl Iterator<Item = u8> {
    (0..count).map(move |i| ((seed as u32 * (i as u32 + 7)) ^ (i as u32 * 31)) as u8)
}
