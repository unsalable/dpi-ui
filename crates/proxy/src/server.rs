//! Yerel proxy sunucusu: HTTP CONNECT, duz HTTP vekil istekleri, SOCKS5 ve PAC dosyasi
//! ayni portta. Yalnizca geri dongu adresini (127.0.0.1) dinler.
//!
//! Her baglanti iki is parcacigiyla tasinir (yon basina bir tane). Sunucudan istemciye
//! yon once baslar; boylece once sunucunun konustugu protokoller de calisir. Istemcinin
//! ilk mesaji `desync::plan` ile parcalanip gonderilir, gerisi oldugu gibi aktarilir.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use socket2::{SockRef, TcpKeepalive};

use crate::desync::{self, Desync};
use crate::resolve::Resolve;

const STACK: usize = 128 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Ilk mesajin geri kalani en fazla bu kadar beklenir (TLS kaydi parcali geldiyse).
const FIRST_REST_WAIT: Duration = Duration::from_millis(150);
const MAX_HEAD: usize = 16 * 1024;
/// Bir yon kapandiktan sonra diger yon bu kadar sessiz kalirsa baglanti kapatilir.
const HALF_CLOSED_IDLE: Duration = Duration::from_secs(60);
const MAX_ADDRS: usize = 4;
const BUF: usize = 16 * 1024;

/// Varsayilan port (Mac'te PAC adresi sabit kalsin diye).
pub const DEFAULT_PORT: u16 = 47813;
pub const PAC_PATH: &str = "/proxy.pac";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Hedefe baglanildi.
    Opened { host: String, port: u16, addr: SocketAddr },
    /// Ilk mesaj bu kadar parca halinde gitti.
    Desynced { host: String, segments: usize },
    Failed { host: String, port: u16, error: String },
}

pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

pub struct Options {
    pub listen: SocketAddr,
    pub desync: Desync,
    pub resolver: Arc<dyn Resolve>,
    pub max_connections: usize,
    pub events: Option<EventSink>,
}

impl Options {
    pub fn new(port: u16, desync: Desync, resolver: Arc<dyn Resolve>) -> Self {
        Self { listen: SocketAddr::from((Ipv4Addr::LOCALHOST, port)), desync, resolver, max_connections: 512, events: None }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub accepted: u64,
    pub active: u64,
    pub desynced: u64,
    pub failed: u64,
    pub rejected: u64,
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    desynced: AtomicU64,
    failed: AtomicU64,
    rejected: AtomicU64,
}

struct Shared {
    addr: SocketAddr,
    desync: RwLock<Arc<Desync>>,
    pac: RwLock<Option<Arc<str>>>,
    resolver: Arc<dyn Resolve>,
    events: Option<EventSink>,
    stop: AtomicBool,
    active: AtomicUsize,
    max: usize,
    counters: Counters,
}

impl Shared {
    fn emit(&self, e: Event) {
        if let Some(sink) = &self.events {
            sink(e);
        }
    }
}

pub struct Proxy {
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

impl Proxy {
    pub fn start(opts: Options) -> io::Result<Self> {
        if !opts.listen.ip().is_loopback() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "proxy yalnızca 127.0.0.1'i dinler"));
        }
        let listener = TcpListener::bind(opts.listen)?;
        let shared = Arc::new(Shared {
            addr: listener.local_addr()?,
            desync: RwLock::new(Arc::new(opts.desync)),
            pac: RwLock::new(None),
            resolver: opts.resolver,
            events: opts.events,
            stop: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            max: opts.max_connections.max(1),
            counters: Counters::default(),
        });
        let s = shared.clone();
        let accept = thread::Builder::new().name("proxy-accept".into()).stack_size(STACK).spawn(move || accept_loop(listener, s))?;
        Ok(Self { shared, accept: Some(accept) })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.shared.addr
    }

    /// Yeni baglantilar bu yontemle gider; acik olanlar etkilenmez.
    pub fn set_desync(&self, d: Desync) {
        *self.shared.desync.write().unwrap() = Arc::new(d);
    }

    /// `GET /proxy.pac` yaniti. `None`: 404.
    pub fn set_pac(&self, script: Option<String>) {
        *self.shared.pac.write().unwrap() = script.map(Into::into);
    }

    pub fn stats(&self) -> Stats {
        let c = &self.shared.counters;
        Stats {
            accepted: c.accepted.load(Ordering::Relaxed),
            active: self.shared.active.load(Ordering::Relaxed) as u64,
            desynced: c.desynced.load(Ordering::Relaxed),
            failed: c.failed.load(Ordering::Relaxed),
            rejected: c.rejected.load(Ordering::Relaxed),
        }
    }

    /// Dinlemeyi birakir. Acik tuneller kendi kapanana kadar surer.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(h) = self.accept.take() {
            self.shared.stop.store(true, Ordering::SeqCst);
            // accept() uyansin.
            let _ = TcpStream::connect_timeout(&self.shared.addr, Duration::from_secs(1));
            let _ = h.join();
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Is parcacigi bitince (panik dahil) etkin sayaci duser.
struct ActiveGuard(Arc<Shared>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_loop(listener: TcpListener, s: Arc<Shared>) {
    for conn in listener.incoming() {
        if s.stop.load(Ordering::SeqCst) {
            // Uyandiran baglanti once karsi taraftan kapansin: TIME_WAIT dinlenen portta
            // kalmasin (Windows'ta ayni port hemen yeniden acilabilsin).
            if let Ok(mut c) = conn {
                let _ = c.set_read_timeout(Some(Duration::from_millis(200)));
                let _ = c.read(&mut [0u8; 1]);
            }
            break;
        }
        let client = match conn {
            Ok(c) => c,
            Err(_) => {
                // Dosya tanitici siniri gibi gecici hatalarda donguyu dondurme.
                thread::sleep(Duration::from_millis(20));
                continue;
            }
        };
        if s.active.load(Ordering::SeqCst) >= s.max {
            s.counters.rejected.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        s.active.fetch_add(1, Ordering::SeqCst);
        s.counters.accepted.fetch_add(1, Ordering::Relaxed);
        let guard = ActiveGuard(s.clone());
        let _ = thread::Builder::new().name("proxy-conn".into()).stack_size(STACK).spawn(move || {
            let g = guard;
            handle(client, &g.0);
        });
    }
}

struct Upstream {
    stream: TcpStream,
    host: String,
    /// Istemciden okunmus, sunucuya gidecek ilk baytlar (yeniden yazilmis HTTP istegi ya
    /// da CONNECT basligindan sonra gelenler).
    pending: Vec<u8>,
}

fn handle(mut client: TcpStream, s: &Shared) {
    let _ = client.set_nodelay(true);
    let _ = client.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
    let mut first = [0u8; 1];
    if !matches!(client.peek(&mut first), Ok(1)) {
        return;
    }
    let upstream = if first[0] == 5 { socks5(&mut client, s) } else { http(&mut client, s) };
    if let Some(up) = upstream {
        tunnel(client, up, s);
    }
}

// ------------------------------------------------------------------- HTTP

fn http(client: &mut TcpStream, s: &Shared) -> Option<Upstream> {
    let (buf, head_end) = read_head(client).ok()?;
    let head = std::str::from_utf8(&buf[..head_end]).ok();
    let Some((method, target, version)) = head.and_then(|h| h.split("\r\n").next()).and_then(|line| {
        let mut p = line.split(' ');
        Some((p.next()?, p.next()?, p.next()?))
    }) else {
        let _ = client.write_all(status(400, "Bad Request").as_bytes());
        return None;
    };

    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = split_host_port(target, 443) else {
            let _ = client.write_all(status(400, "Bad Request").as_bytes());
            return None;
        };
        return match connect(&host, port, s) {
            Ok(stream) => {
                client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").ok()?;
                Some(Upstream { stream, host, pending: buf[head_end..].to_vec() })
            }
            Err(e) => {
                let _ = client.write_all(status(502, "Bad Gateway").as_bytes());
                fail(s, host, port, &e);
                None
            }
        };
    }

    if target.starts_with('/') {
        let pac = s.pac.read().unwrap().clone();
        let path = target.split('?').next().unwrap_or(target);
        let reply = match pac {
            Some(script) if path == PAC_PATH && (method == "GET" || method == "HEAD") => {
                let body = if method == "HEAD" { "" } else { &script };
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
                    script.len()
                )
            }
            _ => status(404, "Not Found"),
        };
        let _ = client.write_all(reply.as_bytes());
        return None;
    }

    // Mutlak bicim: "GET http://alan/yol HTTP/1.1".
    let Some(rest) = target.strip_prefix("http://").or_else(|| target.strip_prefix("HTTP://")) else {
        let _ = client.write_all(status(400, "Bad Request").as_bytes());
        return None;
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let Some((host, port)) = split_host_port(authority, 80) else {
        let _ = client.write_all(status(400, "Bad Request").as_bytes());
        return None;
    };
    let mut out = format!("{method} {path} {version}\r\n");
    let mut has_host = false;
    for line in head?.split("\r\n").skip(1).filter(|l| !l.is_empty()) {
        let name = line.split(':').next().unwrap_or("").trim();
        if ["proxy-connection", "proxy-authorization", "connection", "keep-alive"].iter().any(|h| name.eq_ignore_ascii_case(h)) {
            continue;
        }
        has_host |= name.eq_ignore_ascii_case("host");
        out.push_str(line);
        out.push_str("\r\n");
    }
    if !has_host {
        out.push_str(&format!("Host: {authority}\r\n"));
    }
    // Ayni baglantida baska siteye istek gelmesin: her istek kendi baglantisinda.
    out.push_str("Connection: close\r\n\r\n");
    let mut pending = out.into_bytes();
    pending.extend_from_slice(&buf[head_end..]);
    match connect(&host, port, s) {
        Ok(stream) => Some(Upstream { stream, host, pending }),
        Err(e) => {
            let _ = client.write_all(status(502, "Bad Gateway").as_bytes());
            fail(s, host, port, &e);
            None
        }
    }
}

fn status(code: u16, text: &str) -> String {
    format!("HTTP/1.1 {code} {text}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

/// Basligi "\r\n\r\n"e kadar okur. Doner: (okunanlar, basligin bittigi konum).
fn read_head(client: &mut TcpStream) -> io::Result<(Vec<u8>, usize)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let n = client.read(&mut chunk)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            return Ok((buf, from + i + 4));
        }
        if buf.len() > MAX_HEAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "başlık çok uzun"));
        }
    }
}

/// "alan:port", "[v6]:port" ya da porttsuz "alan".
fn split_host_port(s: &str, default_port: u16) -> Option<(String, u16)> {
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let (h, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None if after.is_empty() => default_port,
            None => return None,
        };
        (h, port)
    } else {
        match s.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().ok()?),
            None => (s, default_port),
        }
    };
    let host = host.trim_end_matches('.');
    if host.is_empty() || port == 0 {
        return None;
    }
    Some((host.to_ascii_lowercase(), port))
}

// ----------------------------------------------------------------- SOCKS5

fn socks5(c: &mut TcpStream, s: &Shared) -> Option<Upstream> {
    let mut hello = [0u8; 2];
    c.read_exact(&mut hello).ok()?;
    let mut methods = vec![0u8; hello[1] as usize];
    c.read_exact(&mut methods).ok()?;
    if !methods.contains(&0) {
        let _ = c.write_all(&[5, 0xFF]);
        return None;
    }
    c.write_all(&[5, 0]).ok()?;

    let mut req = [0u8; 4];
    c.read_exact(&mut req).ok()?;
    if req[0] != 5 {
        return None;
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            c.read_exact(&mut a).ok()?;
            Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            c.read_exact(&mut len).ok()?;
            let mut name = vec![0u8; len[0] as usize];
            c.read_exact(&mut name).ok()?;
            String::from_utf8(name).ok()?.to_ascii_lowercase()
        }
        4 => {
            let mut a = [0u8; 16];
            c.read_exact(&mut a).ok()?;
            Ipv6Addr::from(a).to_string()
        }
        _ => {
            let _ = c.write_all(&socks_reply(8));
            return None;
        }
    };
    let mut port = [0u8; 2];
    c.read_exact(&mut port).ok()?;
    let port = u16::from_be_bytes(port);
    if req[1] != 1 {
        // Yalnizca CONNECT; BIND ve UDP ASSOCIATE yok.
        let _ = c.write_all(&socks_reply(7));
        return None;
    }
    match connect(&host, port, s) {
        Ok(stream) => {
            c.write_all(&socks_reply(0)).ok()?;
            Some(Upstream { stream, host, pending: Vec::new() })
        }
        Err(e) => {
            let code = match e.kind() {
                io::ErrorKind::ConnectionRefused => 5,
                io::ErrorKind::NotFound | io::ErrorKind::TimedOut => 4,
                _ => 1,
            };
            let _ = c.write_all(&socks_reply(code));
            fail(s, host, port, &e);
            None
        }
    }
}

fn socks_reply(code: u8) -> [u8; 10] {
    [5, code, 0, 1, 0, 0, 0, 0, 0, 0]
}

// ---------------------------------------------------------------- hedefe baglanti

fn connect(host: &str, port: u16, s: &Shared) -> io::Result<TcpStream> {
    let mut addrs = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => s.resolver.resolve(host)?,
    };
    // IPv6'si yarim calisan hatlar cok; once IPv4.
    addrs.sort_by_key(|ip| ip.is_ipv6());
    let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{host}: adres yok"));
    for ip in addrs.into_iter().take(MAX_ADDRS) {
        let addr = SocketAddr::new(ip, port);
        if ip.is_unspecified() || (ip.is_loopback() && port == s.addr.port()) {
            last = io::Error::new(io::ErrorKind::InvalidInput, "proxy kendine bağlanamaz");
            continue;
        }
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                let keepalive = TcpKeepalive::new().with_time(Duration::from_secs(60)).with_interval(Duration::from_secs(15));
                let _ = SockRef::from(&stream).set_tcp_keepalive(&keepalive);
                s.emit(Event::Opened { host: host.to_owned(), port, addr });
                return Ok(stream);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn fail(s: &Shared, host: String, port: u16, e: &io::Error) {
    s.counters.failed.fetch_add(1, Ordering::Relaxed);
    s.emit(Event::Failed { host, port, error: e.to_string() });
}

// ------------------------------------------------------------------ tasima

fn tunnel(client: TcpStream, up: Upstream, s: &Shared) {
    let _ = client.set_read_timeout(None);
    let (Ok(mut c_out), Ok(mut u_in)) = (client.try_clone(), up.stream.try_clone()) else { return };
    let started = Instant::now();
    let activity = Arc::new(AtomicU64::new(0));
    let (to_client_done, rx_to_client) = mpsc::channel::<()>();
    let (to_server_done, rx_to_server) = mpsc::channel::<()>();

    let act = activity.clone();
    let back = thread::Builder::new().name("proxy-down".into()).stack_size(STACK).spawn(move || {
        pump(&mut u_in, &mut c_out, &act, started);
        let _ = c_out.shutdown(Shutdown::Write);
        drop(to_client_done);
        wait_or_close(&rx_to_server, &act, started, &[&c_out, &u_in]);
    });
    let Ok(back) = back else { return };

    let mut client = client;
    let mut upstream = up.stream;
    if send_first(&mut client, &mut upstream, up.pending, &up.host, s).is_ok() {
        pump(&mut client, &mut upstream, &activity, started);
    }
    let _ = upstream.shutdown(Shutdown::Write);
    drop(to_server_done);
    wait_or_close(&rx_to_client, &activity, started, &[&client, &upstream]);
    let _ = back.join();
}

/// Diger yonun bitmesini bekler; bir yon kapandiktan sonra baglanti uzun sure sessiz
/// kalirsa iki soketi de kapatir (yarim kapali baglantilar is parcacigi birakmasin).
fn wait_or_close(other_done: &mpsc::Receiver<()>, activity: &AtomicU64, started: Instant, socks: &[&TcpStream]) {
    loop {
        match other_done.recv_timeout(Duration::from_secs(5)) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                let idle = (started.elapsed().as_millis() as u64).saturating_sub(activity.load(Ordering::Relaxed));
                if idle >= HALF_CLOSED_IDLE.as_millis() as u64 {
                    for sock in socks {
                        let _ = sock.shutdown(Shutdown::Both);
                    }
                    return;
                }
            }
        }
    }
}

fn pump(from: &mut TcpStream, to: &mut TcpStream, activity: &AtomicU64, started: Instant) {
    let mut buf = vec![0u8; BUF];
    loop {
        match from.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    return;
                }
                activity.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// Istemcinin ilk mesajini toplar, yonteme gore parcalayip gonderir.
fn send_first(client: &mut TcpStream, up: &mut TcpStream, mut first: Vec<u8>, host: &str, s: &Shared) -> io::Result<()> {
    let mut chunk = vec![0u8; BUF];
    if first.is_empty() {
        let n = loop {
            match client.read(&mut chunk) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                other => break other?,
            }
        };
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        first.extend_from_slice(&chunk[..n]);
    }
    // ClientHello birden fazla okumada geldiyse kaydin tamamini kisa sure bekle.
    if let Some(want) = desync::wanted_len(&first) {
        if want > first.len() {
            client.set_read_timeout(Some(FIRST_REST_WAIT))?;
            while first.len() < want {
                match client.read(&mut chunk[..BUF.min(want - first.len())]) {
                    Ok(0) => break,
                    Ok(n) => first.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            client.set_read_timeout(None)?;
        }
    }

    let d = s.desync.read().unwrap().clone();
    let plan = desync::plan(&first, &d);
    if plan.is_plain() {
        return up.write_all(&plan.data);
    }
    s.counters.desynced.fetch_add(1, Ordering::Relaxed);
    s.emit(Event::Desynced { host: host.to_owned(), segments: plan.segments.len() });
    let v6 = up.peer_addr()?.is_ipv6();
    let sock = SockRef::from(&*up);
    let Ok(normal) = (if v6 { sock.unicast_hops_v6() } else { sock.ttl_v4() }) else {
        // TTL okunamiyorsa ters sira yapilamaz; parcalar normal TTL ile gider.
        for seg in &plan.segments {
            (&*up).write_all(&plan.data[seg.range.clone()])?;
        }
        return Ok(());
    };
    for seg in &plan.segments {
        if seg.low_ttl {
            set_ttl(&sock, v6, 1)?;
        }
        let r = (&*up).write_all(&plan.data[seg.range.clone()]);
        if seg.low_ttl {
            set_ttl(&sock, v6, normal)?;
        }
        r?;
    }
    Ok(())
}

fn set_ttl(sock: &SockRef<'_>, v6: bool, ttl: u32) -> io::Result<()> {
    if v6 { sock.set_unicast_hops_v6(ttl) } else { sock.set_ttl_v4(ttl) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_forms() {
        assert_eq!(split_host_port("Discord.com:443", 1), Some(("discord.com".into(), 443)));
        assert_eq!(split_host_port("example.com", 80), Some(("example.com".into(), 80)));
        assert_eq!(split_host_port("[2606:4700::1111]:443", 1), Some(("2606:4700::1111".into(), 443)));
        assert_eq!(split_host_port("[::1]", 80), Some(("::1".into(), 80)));
        assert_eq!(split_host_port("a.com:x", 80), None);
        assert_eq!(split_host_port(":443", 80), None);
        assert_eq!(split_host_port("a.com:0", 80), None);
    }
}
