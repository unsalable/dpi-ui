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
pub use doh::DohResolver;

#[cfg(feature = "doh")]
mod doh {
    use std::io;
    use std::net::IpAddr;
    use std::sync::Mutex;
    use std::time::Instant;

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
            let name = host.trim_end_matches('.').to_ascii_lowercase();
            if message::is_local_name(&name) {
                return SystemResolver.resolve(host);
            }
            let found = self.lookup(&name, A).and_then(|v4| if v4.is_empty() { self.lookup(&name, AAAA) } else { Ok(v4) });
            match found {
                Ok(ips) if !ips.is_empty() => Ok(ips),
                Ok(_) => Err(io::Error::new(io::ErrorKind::NotFound, format!("{name}: adres yok"))),
                Err(_) if self.fallback_to_system => SystemResolver.resolve(host),
                Err(e) => Err(io::Error::other(e)),
            }
        }
    }
}
