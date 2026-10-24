//! Proxy'nin gercek soketlerle uctan uca testleri (yalnizca 127.0.0.1).

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use proxy::{Desync, Event, Options, Proxy, Resolve, presets};

const NAME: &str = "test.example";

struct MapResolver(HashMap<String, Vec<IpAddr>>);

impl Resolve for MapResolver {
    fn resolve(&self, host: &str) -> io::Result<Vec<IpAddr>> {
        self.0.get(host).cloned().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, host.to_owned()))
    }
}

fn resolver() -> Arc<dyn Resolve> {
    let local = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    Arc::new(MapResolver(HashMap::from([
        (NAME.to_owned(), local.clone()),
        (format!("cdn.{NAME}"), local.clone()),
        ("haric.test.example".to_owned(), local),
    ])))
}

fn start(desync: Desync) -> (Proxy, Arc<Mutex<Vec<Event>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let mut opts = Options::new(0, desync, resolver());
    opts.events = Some(Arc::new(move |e| sink.lock().unwrap().push(e)));
    (Proxy::start(opts).unwrap(), events)
}

/// Her baglantida istemcinin yazdiklarini sonuna kadar okur, kanala ve geri istemciye yollar.
fn echo_server() -> (u16, Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut c) = conn else { return };
            let tx = tx.clone();
            thread::spawn(move || {
                c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut got = Vec::new();
                let _ = c.read_to_end(&mut got);
                let _ = c.write_all(&got);
                let _ = c.shutdown(Shutdown::Write);
                let _ = tx.send(got);
            });
        }
    });
    (port, rx)
}

fn client(proxy: &Proxy) -> TcpStream {
    let c = TcpStream::connect(proxy.local_addr()).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    c
}

/// Bos satira kadar okur (HTTP yanit basligi).
fn read_head(c: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        assert_eq!(c.read(&mut b).unwrap(), 1, "yanit yarim: {:?}", String::from_utf8_lossy(&head));
        head.push(b[0]);
    }
    String::from_utf8(head).unwrap()
}

/// TLS kayitlarini birlestirir: sunucunun TLS katmaninin gordugu tek kayitlik ClientHello.
fn merge_records(data: &[u8]) -> Vec<u8> {
    if data.first() != Some(&0x16) {
        return data.to_vec();
    }
    let mut payload = Vec::new();
    let mut pos = 0;
    while pos + 5 <= data.len() && data[pos] == 0x16 {
        let len = u16::from_be_bytes([data[pos + 3], data[pos + 4]]) as usize;
        payload.extend_from_slice(&data[pos + 5..pos + 5 + len]);
        pos += 5 + len;
    }
    let mut out = data[..3].to_vec();
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&data[pos..]);
    out
}

fn hello() -> Vec<u8> {
    engine::fakes::build_client_hello(NAME, 517, false)
}

#[test]
fn connect_tunnel_delivers_every_method_intact() {
    let (port, rx) = echo_server();
    let mut cases: Vec<(&str, Desync)> = presets::METHODS.iter().map(|m| (m.id, m.desync())).collect();
    cases.push(("none", Desync::default()));
    for (id, desync) in cases {
        let (proxy, events) = start(desync.clone());
        let mut c = client(&proxy);
        write!(c, "CONNECT {NAME}:{port} HTTP/1.1\r\nHost: {NAME}:{port}\r\n\r\n").unwrap();
        assert!(read_head(&mut c).starts_with("HTTP/1.1 200"), "{id}");

        let mut sent = hello();
        sent.extend_from_slice(b"ve sonrasi: uygulama verisi");
        c.write_all(&sent).unwrap();
        c.shutdown(Shutdown::Write).unwrap();
        let mut back = Vec::new();
        c.read_to_end(&mut back).unwrap();

        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(merge_records(&got), sent, "{id}: sunucu ayni baytlari okumali");
        assert_eq!(back, got, "{id}: sunucudan istemciye yon");
        let desynced = events.lock().unwrap().iter().any(|e| matches!(e, Event::Desynced { .. }));
        assert_eq!(desynced, !desync.is_noop(), "{id}");
        assert_eq!(proxy.stats().desynced, u64::from(!desync.is_noop()), "{id}");
        proxy.stop();
    }
}

#[test]
fn scope_limits_reshaping_to_listed_sites() {
    let (port, rx) = echo_server();
    let (proxy, events) = start(presets::TLS_RECORD.desync());
    let mut scope = engine::Scope::default();
    scope.targets.insert(NAME);
    scope.excluded.insert("haric.test.example");
    proxy.set_scope(Some(scope));
    let send = |host: &str| {
        let mut c = client(&proxy);
        write!(c, "CONNECT {host}:{port} HTTP/1.1\r\n\r\n").unwrap();
        assert!(read_head(&mut c).starts_with("HTTP/1.1 200"));
        c.write_all(&hello()).unwrap();
        c.shutdown(Shutdown::Write).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap()
    };
    // Listede (alt alan adi da): bolunur. Listede degil ya da istisna: oldugu gibi.
    assert_eq!(send(NAME).len(), hello().len() + 5);
    assert_eq!(send(&format!("cdn.{NAME}")).len(), hello().len() + 5);
    assert_eq!(send("127.0.0.1"), hello(), "listede olmayan site");
    assert_eq!(send("haric.test.example"), hello(), "istisna");
    assert_eq!(events.lock().unwrap().iter().filter(|e| matches!(e, Event::Desynced { .. })).count(), 2);
}

#[test]
fn hello_written_in_pieces_is_collected_before_splitting() {
    let (port, rx) = echo_server();
    let (proxy, _) = start(presets::TLS_RECORD.desync());
    let mut c = client(&proxy);
    write!(c, "CONNECT 127.0.0.1:{port} HTTP/1.1\r\n\r\n").unwrap();
    assert!(read_head(&mut c).starts_with("HTTP/1.1 200"));
    let h = hello();
    c.write_all(&h[..20]).unwrap();
    thread::sleep(Duration::from_millis(30));
    c.write_all(&h[20..]).unwrap();
    c.shutdown(Shutdown::Write).unwrap();
    let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(got.len(), h.len() + 5, "kayit bolunmus olmali (iki basin tek mesaj sayildi)");
    assert_eq!(merge_records(&got), h);
}

#[test]
fn socks5_with_domain_and_ipv4() {
    let (port, rx) = echo_server();
    let (proxy, _) = start(presets::DISORDER.desync());
    for target in [None, Some([127u8, 0, 0, 1])] {
        let mut c = client(&proxy);
        c.write_all(&[5, 1, 0]).unwrap();
        let mut r = [0u8; 2];
        c.read_exact(&mut r).unwrap();
        assert_eq!(r, [5, 0]);
        let mut req = vec![5, 1, 0];
        match target {
            None => {
                req.extend_from_slice(&[3, NAME.len() as u8]);
                req.extend_from_slice(NAME.as_bytes());
            }
            Some(ip) => {
                req.push(1);
                req.extend_from_slice(&ip);
            }
        }
        req.extend_from_slice(&port.to_be_bytes());
        c.write_all(&req).unwrap();
        let mut reply = [0u8; 10];
        c.read_exact(&mut reply).unwrap();
        assert_eq!(reply[..2], [5, 0]);
        c.write_all(&hello()).unwrap();
        c.shutdown(Shutdown::Write).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), hello());
    }

    // Bilinmeyen ad: "host unreachable".
    let mut c = client(&proxy);
    c.write_all(&[5, 1, 0]).unwrap();
    let mut r = [0u8; 2];
    c.read_exact(&mut r).unwrap();
    let mut req = vec![5, 1, 0, 3, 7];
    req.extend_from_slice(b"yok.yok");
    req.extend_from_slice(&443u16.to_be_bytes());
    c.write_all(&req).unwrap();
    let mut reply = [0u8; 10];
    c.read_exact(&mut reply).unwrap();
    assert_eq!(reply[1], 4);
}

#[test]
fn plain_http_request_is_rewritten() {
    let (port, rx) = echo_server();
    let (proxy, _) = start(presets::DISORDER_SNI.desync());
    let mut c = client(&proxy);
    write!(
        c,
        "GET http://{NAME}:{port}/a?b=1 HTTP/1.1\r\nHost: {NAME}:{port}\r\nProxy-Connection: keep-alive\r\nConnection: keep-alive\r\nAccept: */*\r\n\r\n"
    )
    .unwrap();
    c.shutdown(Shutdown::Write).unwrap();
    let got = String::from_utf8(rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    assert!(got.starts_with("GET /a?b=1 HTTP/1.1\r\n"), "{got}");
    assert!(got.contains(&format!("\r\nhoSt: {NAME}:{port}\r\n")), "{got}");
    assert!(got.contains("\r\nAccept: */*\r\n") && got.ends_with("Connection: close\r\n\r\n"), "{got}");
    assert!(!got.to_ascii_lowercase().contains("keep-alive"), "{got}");
}

#[test]
fn server_that_speaks_first_reaches_the_client() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.write_all(b"220 hazir\r\n").unwrap();
        thread::sleep(Duration::from_millis(300));
    });
    let (proxy, _) = start(presets::DISORDER.desync());
    let mut c = client(&proxy);
    write!(c, "CONNECT {NAME}:{port} HTTP/1.1\r\n\r\n").unwrap();
    assert!(read_head(&mut c).starts_with("HTTP/1.1 200"));
    let mut banner = [0u8; 11];
    c.read_exact(&mut banner).unwrap();
    assert_eq!(&banner, b"220 hazir\r\n");
}

#[test]
fn serves_pac_and_refuses_bad_targets() {
    let (proxy, _) = start(Desync::default());
    let get = |req: &str| {
        let mut c = client(&proxy);
        c.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        out
    };
    assert!(get("GET /proxy.pac HTTP/1.1\r\nHost: x\r\n\r\n").starts_with("HTTP/1.1 404"));
    proxy.set_pac(Some(proxy::pac::direct()));
    let pac = get("GET /proxy.pac?t=1 HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(pac.starts_with("HTTP/1.1 200") && pac.contains("application/x-ns-proxy-autoconfig") && pac.ends_with(&proxy::pac::direct()), "{pac}");

    let me = proxy.local_addr();
    assert!(get(&format!("CONNECT {me} HTTP/1.1\r\n\r\n")).starts_with("HTTP/1.1 502"), "kendine baglanmaz");
    assert!(get("CONNECT yok.yok:443 HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 502"));
    assert!(get("CONNECT :443 HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert!(get("FOO\r\n\r\n").starts_with("HTTP/1.1 400"));
    assert_eq!(proxy.stats().failed, 2);
}

#[test]
fn listens_only_on_loopback_and_stop_closes_the_port() {
    let mut opts = Options::new(0, Desync::default(), resolver());
    opts.listen = SocketAddr::from(([0, 0, 0, 0], 0));
    assert!(Proxy::start(opts).is_err(), "agdaki baska cihazlara acik proxy olmaz");

    let (proxy, _) = start(Desync::default());
    let addr = proxy.local_addr();
    assert!(addr.ip().is_loopback());
    proxy.stop();
    assert!(TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_err());
}
