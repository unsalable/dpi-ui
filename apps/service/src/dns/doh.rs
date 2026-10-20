//! Sifreli DNS istemcisi: DNS over HTTPS (RFC 8484, HTTP/1.1 POST) ya da
//! DNS over TLS (RFC 7858, port 853, 2 bayt uzunluk onekli mesaj). Bazi
//! saglayicilar DoH'ta yalnizca HTTP/2 kabul ettigi icin onlarda DoT kullanilir.
//!
//! Baslangic (bootstrap) IP'lerine dogrudan TLS ile baglanir; SNI ve sertifika
//! dogrulamasi DoH sunucusunun adiyla yapilir, yani DNS cozumlemesine gerek yoktur.
//! En fazla 4 bagli baglanti havuzda tutulur; kopan baglanti sessizce yenilenir.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

const POOL: usize = 4;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 64 * 1024;

type Conn = StreamOwned<ClientConnection, TcpStream>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Https,
    Tls,
}

pub struct DohClient {
    mode: Mode,
    host: String,
    path: String,
    port: u16,
    addrs: Vec<IpAddr>,
    next: AtomicUsize,
    tls: Arc<ClientConfig>,
    server_name: ServerName<'static>,
    pool: Mutex<Vec<Conn>>,
}

impl DohClient {
    /// `https://host/yol` (DoH) ya da `tls://host[:port]` (DoT).
    pub fn new(url: &str, bootstrap: &[IpAddr]) -> Result<Self, String> {
        if let Some(rest) = url.strip_prefix("tls://") {
            let (host, port) = match rest.trim_end_matches('/').rsplit_once(':') {
                Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) => (h, p.parse().map_err(|_| "geçersiz port")?),
                _ => (rest.trim_end_matches('/'), 853),
            };
            return Self::build(Mode::Tls, host, String::new(), port, bootstrap);
        }
        let rest = url.strip_prefix("https://").ok_or("DoH adresi https:// ile başlamalı")?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], rest[i..].to_owned()),
            None => (rest, "/dns-query".to_owned()),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) => (h, p.parse().map_err(|_| "geçersiz port")?),
            _ => (authority, 443),
        };
        Self::build(Mode::Https, host, path, port, bootstrap)
    }

    fn build(mode: Mode, host: &str, path: String, port: u16, bootstrap: &[IpAddr]) -> Result<Self, String> {
        if bootstrap.is_empty() {
            return Err("DNS sunucusu için başlangıç IP adresi yok".into());
        }
        let server_name = ServerName::try_from(host.to_owned()).map_err(|_| "geçersiz DoH sunucu adı")?;

        let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let mut tls = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        if mode == Mode::Https {
            tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        }

        Ok(Self {
            mode,
            host: host.to_owned(),
            path,
            port,
            addrs: bootstrap.to_vec(),
            next: AtomicUsize::new(0),
            tls: Arc::new(tls),
            server_name,
            pool: Mutex::new(Vec::with_capacity(POOL)),
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// Ham DNS sorgusunu gonderir, ham yaniti dondurur.
    pub fn resolve(&self, query: &[u8]) -> Result<Vec<u8>, String> {
        let mut last = String::from("bağlantı kurulamadı");
        for attempt in 0..3 {
            // Ilk deneme havuzdan; havuzdaki baglanti sunucuca kapatilmis olabilir.
            let pooled = if attempt == 0 { self.pool.lock().unwrap().pop() } else { None };
            let mut conn = match pooled {
                Some(c) => c,
                None => match self.connect() {
                    Ok(c) => c,
                    Err(e) => {
                        last = e;
                        continue;
                    }
                },
            };
            match self.exchange(&mut conn, query) {
                Ok((body, keep)) => {
                    if keep {
                        let mut pool = self.pool.lock().unwrap();
                        if pool.len() < POOL {
                            pool.push(conn);
                        }
                    }
                    return Ok(body);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Havuzu bosaltir (ag degisince eski baglantilar oluk olabilir).
    pub fn reset(&self) {
        self.pool.lock().unwrap().clear();
    }

    fn connect(&self) -> Result<Conn, String> {
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        let mut last = String::new();
        for i in 0..self.addrs.len() {
            let ip = self.addrs[(start + i) % self.addrs.len()];
            let addr = SocketAddr::new(ip, self.port);
            match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
                Ok(sock) => {
                    let _ = sock.set_nodelay(true);
                    let _ = sock.set_read_timeout(Some(IO_TIMEOUT));
                    let _ = sock.set_write_timeout(Some(IO_TIMEOUT));
                    let conn = ClientConnection::new(self.tls.clone(), self.server_name.clone()).map_err(|e| e.to_string())?;
                    return Ok(StreamOwned::new(conn, sock));
                }
                Err(e) => last = format!("{addr}: {e}"),
            }
        }
        Err(last)
    }

    fn exchange(&self, conn: &mut Conn, query: &[u8]) -> Result<(Vec<u8>, bool), String> {
        if self.mode == Mode::Tls {
            return exchange_dot(conn, query).map(|r| (r, true));
        }
        let head = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/dns-message\r\nAccept: application/dns-message\r\nContent-Length: {}\r\n\r\n",
            self.path,
            self.host,
            query.len()
        );
        let mut req = Vec::with_capacity(head.len() + query.len());
        req.extend_from_slice(head.as_bytes());
        req.extend_from_slice(query);
        conn.write_all(&req).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;

        // Baslik.
        let mut buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 2048];
        let header_end = loop {
            if let Some(i) = find(&buf, b"\r\n\r\n") {
                break i + 4;
            }
            if buf.len() > MAX_HEADER {
                return Err("HTTP başlığı çok büyük".into());
            }
            let n = conn.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("sunucu bağlantıyı kapattı".into());
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let header = std::str::from_utf8(&buf[..header_end]).map_err(|_| "geçersiz HTTP başlığı")?.to_ascii_lowercase();
        let mut lines = header.split("\r\n");
        let status = lines.next().unwrap_or("");
        if !status.starts_with("http/1.1 200") && !status.starts_with("http/1.0 200") {
            return Err(format!("DoH sunucusu yanıtı: {}", status.trim()));
        }
        let mut length = None;
        let mut chunked = false;
        let mut keep = true;
        for l in lines {
            if let Some(v) = l.strip_prefix("content-length:") {
                length = v.trim().parse::<usize>().ok();
            } else if let Some(v) = l.strip_prefix("transfer-encoding:") {
                chunked = v.contains("chunked");
            } else if let Some(v) = l.strip_prefix("connection:") {
                keep = !v.contains("close");
            }
        }

        let mut body = buf.split_off(header_end);
        if chunked {
            loop {
                if let Some(decoded) = decode_chunked(&body)? {
                    return Ok((decoded, keep));
                }
                if body.len() > MAX_BODY {
                    return Err("DoH yanıtı çok büyük".into());
                }
                let n = conn.read(&mut chunk).map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("yanıt yarıda kesildi".into());
                }
                body.extend_from_slice(&chunk[..n]);
            }
        }
        let len = length.ok_or("yanıt uzunluğu yok")?;
        if len > MAX_BODY {
            return Err("DoH yanıtı çok büyük".into());
        }
        while body.len() < len {
            let n = conn.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("yanıt yarıda kesildi".into());
            }
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(len);
        Ok((body, keep))
    }
}

/// DoT: 2 bayt uzunluk + mesaj; yanit da ayni bicimde.
fn exchange_dot(conn: &mut Conn, query: &[u8]) -> Result<Vec<u8>, String> {
    let mut req = Vec::with_capacity(2 + query.len());
    req.extend_from_slice(&(query.len() as u16).to_be_bytes());
    req.extend_from_slice(query);
    conn.write_all(&req).map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    let mut len = [0u8; 2];
    conn.read_exact(&mut len).map_err(|e| e.to_string())?;
    let len = u16::from_be_bytes(len) as usize;
    if len < 12 {
        return Err("geçersiz DoT yanıtı".into());
    }
    let mut body = vec![0u8; len];
    conn.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Tamamlanmis "chunked" govdeyi cozer; eksikse `Ok(None)`.
fn decode_chunked(mut b: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut out = Vec::new();
    loop {
        let Some(line_end) = find(b, b"\r\n") else { return Ok(None) };
        let size_text = std::str::from_utf8(&b[..line_end]).map_err(|_| "geçersiz parça")?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| "geçersiz parça boyu")?;
        b = &b[line_end + 2..];
        if size == 0 {
            return Ok(Some(out));
        }
        if b.len() < size + 2 {
            return Ok(None);
        }
        out.extend_from_slice(&b[..size]);
        if out.len() > MAX_BODY {
            return Err("DoH yanıtı çok büyük".into());
        }
        b = &b[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        let c = DohClient::new("https://dns.example:8443/q", &["9.9.9.9".parse().unwrap()]).unwrap();
        assert_eq!((c.host.as_str(), c.port, c.path.as_str()), ("dns.example", 8443, "/q"));
        let d = DohClient::new("https://cloudflare-dns.com", &["1.1.1.1".parse().unwrap()]).unwrap();
        assert_eq!(d.path, "/dns-query");
        assert!(DohClient::new("http://x", &["1.1.1.1".parse().unwrap()]).is_err());
        let t = DohClient::new("tls://dns.quad9.net", &["9.9.9.9".parse().unwrap()]).unwrap();
        assert_eq!((t.mode, t.host.as_str(), t.port), (Mode::Tls, "dns.quad9.net", 853));
        assert!(DohClient::new("https://x.example/q", &[]).is_err());
    }

    #[test]
    fn decodes_chunked() {
        assert_eq!(decode_chunked(b"4\r\nwiki\r\n5\r\npedia\r\n0\r\n\r\n").unwrap(), Some(b"wikipedia".to_vec()));
        assert_eq!(decode_chunked(b"4\r\nwi").unwrap(), None);
        assert!(decode_chunked(b"zz\r\n").is_err());
    }
}
