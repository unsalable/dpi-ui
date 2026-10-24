//! Proxy'nin acacagi adlarin cozumu. Turkiye'de engelli adlar sistem DNS'inde yanlis
//! adrese cozuldugu icin varsayilan sifreli DNS'tir; sistem DNS'i yalnizca yedek.

use std::io;
use std::net::{IpAddr, ToSocketAddrs};

pub trait Resolve: Send + Sync {
    /// Adin adresleri; bos liste yerine hata doner.
    fn resolve(&self, host: &str) -> io::Result<Vec<IpAddr>>;
}

/// Isletim sisteminin cozucusu.
pub struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, host: &str) -> io::Result<Vec<IpAddr>> {
        let ips: Vec<IpAddr> = (host, 0).to_socket_addrs()?.map(|a| a.ip()).collect();
        if ips.is_empty() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("{host}: adres yok")));
        }
        Ok(ips)
    }
}

#[cfg(feature = "doh")]
pub use doh::{DohResolver, UdpResolver};

#[cfg(feature = "doh")]
mod doh {
    use std::io;
    use std::net::{IpAddr, SocketAddr, UdpSocket};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use dnsclient::cache::{Cache, NEGATIVE_TTL};
    use dnsclient::doh::DohClient;
    use dnsclient::message;

    use super::{Resolve, SystemResolver};

    const CACHE_ENTRIES: usize = 512;
    const A: u16 = 1;
    const AAAA: u16 = 28;

    pub struct DohResolver {
        client: DohClient,
        cache: Mutex<Cache>,
        fallback_to_system: bool,
    }

    impl DohResolver {
        /// `url`: `https://...` (DoH) ya da `tls://...` (DoT); `bootstrap`: sunucunun IP'leri.
        pub fn new(url: &str, bootstrap: &[IpAddr], fallback_to_system: bool) -> Result<Self, String> {
            Ok(Self { client: DohClient::new(url, bootstrap)?, cache: Mutex::new(Cache::new(CACHE_ENTRIES)), fallback_to_system })
        }

        pub fn host(&self) -> &str {
            self.client.host()
        }

        fn lookup(&self, name: &str, qtype: u16) -> Result<Vec<IpAddr>, String> {
            let now = Instant::now();
            if let Some(resp) = self.cache.lock().unwrap().get(name, qtype, 1, now) {
                return Ok(message::answer_addrs(&resp));
            }
            let resp = self.client.resolve(&message::build_query(name, qtype))?;
            let ttl = match message::rcode(&resp) {
                Some(0) => message::min_answer_ttl(&resp).unwrap_or(NEGATIVE_TTL),
                Some(3) => NEGATIVE_TTL,
                other => return Err(format!("DNS yanıt kodu {other:?}")),
            };
            self.cache.lock().unwrap().put(name, qtype, 1, &resp, ttl, now);
            Ok(message::answer_addrs(&resp))
        }
    }

    impl Resolve for DohResolver {
        fn resolve(&self, host: &str) -> io::Result<Vec<IpAddr>> {
            resolve_with(host, self.fallback_to_system, |name, qtype| self.lookup(name, qtype))
        }
    }

    /// Duz DNS (UDP) ile secilen sunucuya sorar: Windows'taki "yonlendirme" kipinin karsiligi
    /// (orn. Yandex 77.88.8.8:1253; ISS'nin 53. porttaki mudahalesini atlar).
    pub struct UdpResolver {
        servers: Vec<SocketAddr>,
        cache: Mutex<Cache>,
        fallback_to_system: bool,
    }

    impl UdpResolver {
        pub fn new(servers: Vec<SocketAddr>, fallback_to_system: bool) -> Result<Self, String> {
            if servers.is_empty() {
                return Err("DNS sunucusu adresi yok".into());
            }
            Ok(Self { servers, cache: Mutex::new(Cache::new(CACHE_ENTRIES)), fallback_to_system })
        }

        fn lookup(&self, name: &str, qtype: u16) -> Result<Vec<IpAddr>, String> {
            let now = Instant::now();
            if let Some(resp) = self.cache.lock().unwrap().get(name, qtype, 1, now) {
                return Ok(message::answer_addrs(&resp));
            }
            let mut query = message::build_query(name, qtype);
            // Sahte yanitlari ayirt etmek icin rastgele gorunen kimlik.
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
            let id = ((nanos >> 4) ^ std::process::id()) as u16 ^ 0x5a5a;
            message::set_id(&mut query, id);
            let mut last = String::from("yanıt yok");
            for server in &self.servers {
                let bind: SocketAddr = if server.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
                let Ok(sock) = UdpSocket::bind(bind) else { continue };
                let _ = sock.set_read_timeout(Some(UDP_TIMEOUT));
                if let Err(e) = sock.send_to(&query, server) {
                    last = e.to_string();
                    continue;
                }
                let mut buf = [0u8; 4096];
                while let Ok((n, from)) = sock.recv_from(&mut buf) {
                    let resp = &buf[..n];
                    if from != *server || n < 12 || resp[..2] != id.to_be_bytes() {
                        continue;
                    }
                    let ttl = match message::rcode(resp) {
                        Some(0) => message::min_answer_ttl(resp).unwrap_or(NEGATIVE_TTL),
                        Some(3) => NEGATIVE_TTL,
                        other => {
                            last = format!("DNS yanıt kodu {other:?}");
                            break;
                        }
                    };
                    self.cache.lock().unwrap().put(name, qtype, 1, resp, ttl, now);
                    return Ok(message::answer_addrs(resp));
                }
            }
            Err(last)
        }
    }

    impl Resolve for UdpResolver {
        fn resolve(&self, host: &str) -> io::Result<Vec<IpAddr>> {
            resolve_with(host, self.fallback_to_system, |name, qtype| self.lookup(name, qtype))
        }
    }

    const UDP_TIMEOUT: Duration = Duration::from_secs(2);

    /// Yerel adlar sistemden; once A, yoksa AAAA; hata olursa istenirse sistem DNS'i.
    fn resolve_with(host: &str, fallback: bool, lookup: impl Fn(&str, u16) -> Result<Vec<IpAddr>, String>) -> io::Result<Vec<IpAddr>> {
        let name = host.trim_end_matches('.').to_ascii_lowercase();
        if message::is_local_name(&name) {
            return SystemResolver.resolve(host);
        }
        let found = lookup(&name, A).and_then(|v4| if v4.is_empty() { lookup(&name, AAAA) } else { Ok(v4) });
        match found {
            Ok(ips) if !ips.is_empty() => Ok(ips),
            Ok(_) => Err(io::Error::new(io::ErrorKind::NotFound, format!("{name}: adres yok"))),
            Err(_) if fallback => SystemResolver.resolve(host),
            Err(e) => Err(io::Error::other(e)),
        }
    }
}
