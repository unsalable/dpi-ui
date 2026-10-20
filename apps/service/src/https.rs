//! Guncelleme icin en kucuk HTTPS GET istemcisi (HTTP/1.1, rustls). Yalnizca GitHub
//! adreslerine gider (yonlendirmeler dahil), boyut ve sure sinirlidir.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HEADER: usize = 32 * 1024;
const MAX_REDIRECTS: usize = 5;

fn tls() -> Result<Arc<ClientConfig>, String> {
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(c) = CFG.get() {
        return Ok(c.clone());
    }
    let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let mut cfg = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(CFG.get_or_init(|| Arc::new(cfg)).clone())
}

struct Url {
    host: String,
    port: u16,
    path: String,
}

fn parse(url: &str) -> Result<Url, String> {
    let rest = url.strip_prefix("https://").ok_or("yalnızca https adreslerine izin var")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().map_err(|_| "geçersiz port")?),
        None => (authority, 443),
    };
    if !shared::update::allowed_host(host) {
        return Err(format!("izin verilmeyen sunucu: {host}"));
    }
    Ok(Url { host: host.to_owned(), port, path: path.to_owned() })
}

/// `url`'yi indirir; govde `max` bayttan buyukse hata. `progress(okunan, toplam)`.
pub fn get(url: &str, accept: &str, max: u64, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        match fetch(&current, accept, max, progress)? {
            Fetched::Body(b) => return Ok(b),
            Fetched::Redirect(to) => current = to,
        }
    }
    Err("çok fazla yönlendirme".into())
}

enum Fetched {
    Body(Vec<u8>),
    Redirect(String),
}

fn fetch(url: &str, accept: &str, max: u64, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Fetched, String> {
    let u = parse(url)?;
    let addr = (u.host.as_str(), u.port)
        .to_socket_addrs()
        .map_err(|e| format!("{} çözümlenemedi: {e}", u.host))?
        .next()
        .ok_or_else(|| format!("{} için adres yok", u.host))?;
    let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(|e| format!("{} bağlantısı: {e}", u.host))?;
    tcp.set_read_timeout(Some(IO_TIMEOUT)).ok();
    tcp.set_write_timeout(Some(IO_TIMEOUT)).ok();
    let name = ServerName::try_from(u.host.clone()).map_err(|_| "geçersiz sunucu adı")?;
    let conn = ClientConnection::new(tls()?, name).map_err(|e| e.to_string())?;
    let mut s = StreamOwned::new(conn, tcp);

    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: GoodbyeDPI-UI-3/{}\r\nAccept: {accept}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        u.path,
        u.host,
        shared::VERSION
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;

    // Baslik.
    let mut head = Vec::with_capacity(4096);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HEADER {
            return Err("yanıt başlığı çok büyük".into());
        }
        match s.read(&mut byte) {
            Ok(0) => return Err("bağlantı erken kapandı".into()),
            Ok(_) => head.push(byte[0]),
            Err(e) => return Err(e.to_string()),
        }
    }
    let head = String::from_utf8_lossy(&head);
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|c| c.parse().ok()).ok_or("geçersiz HTTP yanıtı")?;
    let header = |name: &str| {
        head.split("\r\n").skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    };
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        let to = header("location").ok_or("yönlendirme adresi yok")?;
        return Ok(Fetched::Redirect(to));
    }
    if status != 200 {
        return Err(format!("sunucu {status} döndü"));
    }

    // Govde: chunked, Content-Length ya da baglanti kapanana kadar.
    let chunked = header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let length: Option<u64> = header("content-length").and_then(|v| v.parse().ok());
    if length.is_some_and(|l| l > max) {
        return Err("dosya beklenenden büyük".into());
    }
    let mut body = Vec::with_capacity(length.unwrap_or(64 * 1024).min(max) as usize);
    let mut buf = vec![0u8; 64 * 1024];
    if chunked {
        let mut raw = Vec::new();
        loop {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    raw.extend_from_slice(&buf[..n]);
                    if raw.len() as u64 > max + 64 * 1024 {
                        return Err("dosya beklenenden büyük".into());
                    }
                }
                // TLS kapanis bildirimi olmadan biten sunucular: govde zaten tam olabilir.
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        body = dechunk(&raw)?;
        progress(body.len() as u64, Some(body.len() as u64));
    } else {
        loop {
            if length.is_some_and(|l| body.len() as u64 >= l) {
                break;
            }
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    body.extend_from_slice(&buf[..n]);
                    if body.len() as u64 > max {
                        return Err("dosya beklenenden büyük".into());
                    }
                    progress(body.len() as u64, length);
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        if let Some(l) = length {
            if body.len() as u64 != l {
                return Err("indirme yarıda kaldı".into());
            }
        }
    }
    Ok(Fetched::Body(body))
}

fn dechunk(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    loop {
        let end = raw[i..].windows(2).position(|w| w == b"\r\n").ok_or("bozuk parçalı yanıt")? + i;
        let size_text = String::from_utf8_lossy(&raw[i..end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| "bozuk parça boyutu")?;
        i = end + 2;
        if size == 0 {
            return Ok(out);
        }
        if i + size > raw.len() {
            return Err("parçalı yanıt yarıda kaldı".into());
        }
        out.extend_from_slice(&raw[i..i + size]);
        i += size + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_chunked_bodies() {
        assert_eq!(dechunk(b"4\r\nWiki\r\n6;x=y\r\npedia \r\n0\r\n\r\n").unwrap(), b"Wikipedia ");
        assert!(dechunk(b"ff\r\nkisa").is_err());
    }

    #[test]
    fn refuses_non_github_and_plain_http() {
        assert!(parse("http://api.github.com/x").is_err());
        assert!(parse("https://example.com/x").is_err());
        assert!(parse("https://github.com.evil.example/x").is_err());
        let u = parse("https://api.github.com/repos/a/b/releases?per_page=5").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("api.github.com", 443, "/repos/a/b/releases?per_page=5"));
    }
}
