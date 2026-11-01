//! Kucuk HTTPS istemcisi (HTTP/1.1 GET). Android'de curl yok: "En iyi yontemi bul" istekleri
//! yerel proxy uzerinden (CONNECT), guncelleme denetimi ve indirmesi dogrudan buradan gider.
//! Mac surumu ayni isler icin sistemin curl'unu kullanmaya devam eder.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEAD: usize = 32 * 1024;
const MAX_REDIRECTS: usize = 5;

pub struct Options<'a> {
    /// Istek bu yerel proxy uzerinden (HTTP CONNECT) gider; yoksa dogrudan.
    pub proxy: Option<SocketAddr>,
    /// Istegin tamami (baglanti + govde) en fazla bu kadar surer.
    pub deadline: Duration,
    pub max_body: u64,
    pub headers: &'a [(&'a str, &'a str)],
    /// Yalnizca durum satiri: govde okunmaz (sunucuya ulasildi mi?).
    pub head_only: bool,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self { proxy: None, deadline: Duration::from_secs(20), max_body: 4 * 1024 * 1024, headers: &[], head_only: false }
    }
}

#[derive(Debug)]
pub struct Head {
    pub status: u16,
    pub location: Option<String>,
}

fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            let mut tls = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("TLS surumleri")
                .with_root_certificates(roots)
                .with_no_client_auth();
            tls.alpn_protocols = vec![b"http/1.1".to_vec()];
            Arc::new(tls)
        })
        .clone()
}

/// `https://alan[:port]/yol` -> (alan, port, yol).
fn split_url(url: &str) -> Option<(&str, u16, &str)> {
    let rest = url.strip_prefix("https://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (h, p.parse().ok()?),
        _ => (authority, 443),
    };
    (!host.is_empty()).then_some((host, port, path))
}

struct Deadline(Instant);

impl Deadline {
    fn left(&self) -> io::Result<Duration> {
        self.0.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or_else(|| io::ErrorKind::TimedOut.into())
    }
}

fn open(host: &str, port: u16, proxy: Option<SocketAddr>, deadline: &Deadline) -> io::Result<TcpStream> {
    let Some(proxy) = proxy else {
        let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{host}: adres yok"));
        for addr in (host, port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT.min(deadline.left()?)) {
                Ok(s) => return Ok(s),
                Err(e) => last = e,
            }
        }
        return Err(last);
    };
    let mut s = TcpStream::connect_timeout(&proxy, CONNECT_TIMEOUT.min(deadline.left()?))?;
    s.set_read_timeout(Some(deadline.left()?))?;
    write!(s, "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n")?;
    let (head, rest) = read_head(&mut s)?;
    if !rest.is_empty() || !head.split(' ').nth(1).is_some_and(|c| c == "200") {
        return Err(io::Error::other(format!("proxy: {}", head.lines().next().unwrap_or(""))));
    }
    Ok(s)
}

/// Bos satira kadar okur. Doner: (baslik metni, basliktan sonra okunanlar).
fn read_head(r: &mut impl Read) -> io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    loop {
        let n = r.read(&mut chunk)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            let end = from + i + 4;
            let head = String::from_utf8_lossy(&buf[..end]).into_owned();
            return Ok((head, buf[end..].to_vec()));
        }
        if buf.len() > MAX_HEAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "başlık çok uzun"));
        }
    }
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.split("\r\n").skip(1).find_map(|l| l.split_once(':').filter(|(n, _)| n.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim()))
}

/// Tek istek; yonlendirmeleri izlemez. Govde (2xx'te) `sink`e yazilir.
pub fn get(url: &str, opts: &Options<'_>, sink: &mut dyn Write) -> Result<Head, String> {
    request(url, opts, sink).map_err(|e| e.to_string())
}

fn request(url: &str, opts: &Options<'_>, sink: &mut dyn Write) -> io::Result<Head> {
    let deadline = Deadline(Instant::now() + opts.deadline);
    let (host, port, path) = split_url(url).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "yalnızca https:// adresleri"))?;
    let tcp = open(host, port, opts.proxy, &deadline)?;
    let name = ServerName::try_from(host.to_owned()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "geçersiz sunucu adı"))?;
    let conn = ClientConnection::new(tls_config(), name).map_err(io::Error::other)?;
    let mut tls = StreamOwned::new(conn, tcp);
    tls.sock.set_read_timeout(Some(deadline.left()?))?;
    tls.sock.set_write_timeout(Some(deadline.left()?))?;

    let mut req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept-Encoding: identity\r\n");
    if !opts.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("user-agent")) {
        req.push_str("User-Agent: DPI-UI\r\n");
    }
    for (n, v) in opts.headers {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    req.push_str("\r\n");
    tls.write_all(req.as_bytes())?;

    let (head, first) = read_head(&mut tls)?;
    let status = head.split(' ').nth(1).and_then(|c| c.parse::<u16>().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP yanıtı değil"))?;
    let location = header(&head, "location").map(str::to_owned);
    if opts.head_only || !(200..300).contains(&status) {
        return Ok(Head { status, location });
    }

    let chunked = header(&head, "transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let length = header(&head, "content-length").and_then(|v| v.parse::<u64>().ok());
    if length.is_some_and(|l| l > opts.max_body) {
        return Err(io::Error::other("yanıt çok büyük"));
    }
    let limit = if chunked { opts.max_body.saturating_mul(2) } else { length.unwrap_or(opts.max_body) };
    let mut raw: Vec<u8> = Vec::new();
    let mut written = 0u64;
    let mut emit = |data: &[u8], raw: &mut Vec<u8>, written: &mut u64| -> io::Result<()> {
        *written += data.len() as u64;
        if *written > limit {
            return Err(io::Error::other("yanıt çok büyük"));
        }
        if chunked { raw.extend_from_slice(data) } else { sink.write_all(data)? }
        Ok(())
    };
    let take = |n: usize, written: u64| if length.is_some() { n.min((limit - written) as usize) } else { n };
    let first_len = take(first.len(), 0);
    emit(&first[..first_len], &mut raw, &mut written)?;
    let mut buf = vec![0u8; 64 * 1024];
    while length.is_none_or(|l| written < l) {
        tls.sock.set_read_timeout(Some(deadline.left()?))?;
        let n = match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            // Sunucu close_notify yollamadan kapatti: uzunluk bilinmiyorsa govde bitti demektir.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof && length.is_none() => break,
            Err(e) => return Err(e),
        };
        let n = take(n, written);
        emit(&buf[..n], &mut raw, &mut written)?;
    }
    if length.is_some_and(|l| written < l) {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "yanıt yarım kaldı"));
    }
    if chunked {
        let body = dechunk(&raw).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bozuk parçalı yanıt"))?;
        if body.len() as u64 > opts.max_body {
            return Err(io::Error::other("yanıt çok büyük"));
        }
        sink.write_all(&body)?;
    }
    Ok(Head { status, location })
}

/// "Transfer-Encoding: chunked" govdesini cozer.
fn dechunk(mut d: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(d.len());
    loop {
        let line_end = d.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(&d[..line_end]).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        d = &d[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(d.get(..size)?);
        d = d.get(size..)?.strip_prefix(b"\r\n")?;
    }
}

/// Yonlendirmeleri (en fazla 5) izleyen GET. Her adimin sunucusu `allowed` olmali.
pub fn get_following(url: &str, opts: &Options<'_>, allowed: &dyn Fn(&str) -> bool, sink: &mut dyn Write) -> Result<(), String> {
    let mut url = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        let host = split_url(&url).map(|(h, _, _)| h.to_ascii_lowercase()).ok_or_else(|| "yalnızca https:// adresleri".to_owned())?;
        if !allowed(&host) {
            return Err(shared::t!("beklenmeyen indirme adresi", "unexpected download address"));
        }
        let head = get(&url, opts, sink)?;
        match head.status {
            200..=299 => return Ok(()),
            301 | 302 | 303 | 307 | 308 => match head.location {
                Some(next) if next.starts_with("https://") => url = next,
                _ => return Err(format!("HTTP {}", head.status)),
            },
            other => return Err(format!("HTTP {other}")),
        }
    }
    Err("çok fazla yönlendirme".into())
}

/// Yonlendirmeleri izleyerek dosyaya indirir; basarisizsa yarim dosya silinir.
pub fn download(url: &str, dest: &Path, max: u64, allowed: &dyn Fn(&str) -> bool) -> Result<(), String> {
    let result = (|| {
        let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
        let opts = Options { deadline: Duration::from_secs(600), max_body: max, ..Options::default() };
        get_following(url, &opts, allowed, &mut file)?;
        file.sync_all().map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_urls() {
        assert_eq!(split_url("https://discord.com/"), Some(("discord.com", 443, "/")));
        assert_eq!(split_url("https://a.b:8443/x/y?z=1"), Some(("a.b", 8443, "/x/y?z=1")));
        assert_eq!(split_url("https://example.com"), Some(("example.com", 443, "/")));
        assert_eq!(split_url("http://example.com/"), None);
        assert_eq!(split_url("https:///x"), None);
    }

    #[test]
    fn decodes_chunked_bodies() {
        assert_eq!(dechunk(b"4\r\nWiki\r\n6;x=y\r\npedia \r\n0\r\n\r\n").as_deref(), Some(&b"Wikipedia "[..]));
        assert_eq!(dechunk(b"0\r\n\r\n").as_deref(), Some(&b""[..]));
        assert!(dechunk(b"5\r\nab").is_none(), "yarim parca");
        assert!(dechunk(b"zz\r\n").is_none());
    }

    #[test]
    fn reads_headers() {
        let head = "HTTP/1.1 302 Found\r\nLocation: https://x/y\r\nContent-Length: 0\r\n\r\n";
        assert_eq!(header(head, "location"), Some("https://x/y"));
        assert_eq!(header(head, "content-length"), Some("0"));
        assert_eq!(header(head, "etag"), None);
    }

    /// Gercek sunuculara (ag gerekir): `cargo test -p macservice -- --ignored http`.
    #[test]
    #[ignore]
    fn fetches_real_sites() {
        let mut body = Vec::new();
        get_following("https://example.com/", &Options::default(), &|_| true, &mut body).unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Example Domain"));
        let mut json = Vec::new();
        let opts = Options { headers: &[("Accept", "application/vnd.github+json")], ..Options::default() };
        get_following(shared::update::RELEASES_API, &opts, &|_| true, &mut json).unwrap();
        assert!(serde_json::from_slice::<Vec<shared::update::Release>>(&json).is_ok());
        // "En iyi yontemi bul"un Android istegi: yerel proxy uzerinden.
        let p = proxy::Proxy::start(proxy::Options::new(0, proxy::presets::TLS_RECORD.desync(), Arc::new(proxy::SystemResolver))).unwrap();
        assert!(crate::search::native(p.local_addr(), "https://example.com/"));
        assert!(!crate::search::native(p.local_addr(), "https://yok.invalid/"));
    }
}
