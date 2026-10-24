//! "En iyi yontemi bul": yerel bir DPI benzetimine karsi. Benzetim Kablonet'te olculen
//! davranisi taklit eder: TCP parcalarini birlestirir (bolme ve ters sira ise yaramaz) ama
//! TLS kayitlarini birlestirmez; ilk kayitta engelli adin tamami gorunurse baglantiyi keser.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use macservice::search::{self, Fetch};
use macservice::sysproxy::Untouched;
use macservice::Controller;
use proxy::Resolve;
use shared::config::AppConfig;
use shared::i18n::Language;
use shared::ipc::{Request, Response};

const BLOCKED: &[&str] = &["discord.com", "gateway.discord.gg", "www.roblox.com", "www.wattpad.com"];

struct Localhost;

impl Resolve for Localhost {
    fn resolve(&self, _: &str) -> io::Result<Vec<IpAddr>> {
        Ok(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)])
    }
}

/// Ilk TLS kaydinin tamamini (TCP parcalari birlestirilerek) okur; engelli ad o kayitta
/// tam gorunuyorsa keser, gorunmuyorsa "sunucu" yanit verir.
fn dpi_site() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut c) = conn else { return };
            std::thread::spawn(move || {
                c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while buf.len() < 5 || buf.len() < 5 + u16::from_be_bytes([buf[3], buf[4]]) as usize {
                    match c.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let first = &buf[5..5 + u16::from_be_bytes([buf[3], buf[4]]) as usize];
                if BLOCKED.iter().any(|h| first.windows(h.len()).any(|w| w == h.as_bytes())) {
                    let _ = c.shutdown(Shutdown::Both);
                    return;
                }
                let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                // Okunmamis veriyle kapatmak RST gonderir, yanit yolda kaybolabilir: gerisini oku.
                let _ = c.shutdown(Shutdown::Write);
                while matches!(c.read(&mut chunk), Ok(n) if n > 0) {}
            });
        }
    });
    port
}

/// curl yerine: proxy uzerinden CONNECT, adin ClientHello'su, yanit geldiyse acildi.
fn fetch_via(site_port: u16) -> Arc<Fetch> {
    Arc::new(move |proxy: SocketAddr, url: &str| {
        let host = url.trim_start_matches("https://").trim_end_matches('/');
        let Ok(mut c) = TcpStream::connect(proxy) else { return false };
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        if write!(c, "CONNECT {host}:{site_port} HTTP/1.1\r\n\r\n").is_err() {
            return false;
        }
        let mut head = [0u8; 39];
        if c.read_exact(&mut head).is_err() || !head.starts_with(b"HTTP/1.1 200") {
            return false;
        }
        if c.write_all(&engine::fakes::build_client_hello(host, 517, false)).is_err() {
            return false;
        }
        let mut reply = [0u8; 12];
        c.read_exact(&mut reply).is_ok() && &reply == b"HTTP/1.1 200"
    })
}

#[test]
fn picks_the_record_split_against_a_reassembling_dpi() {
    shared::i18n::set(Language::Turkish);
    let out = search::run(Arc::new(Localhost), fetch_via(dpi_site()), false);
    assert!(out.online, "engelsiz site acilmali");
    assert_eq!(out.baseline, 0, "yontemsiz hepsi engelli");
    assert!(out.blocked());
    let opened = |id: &str| out.trials.iter().find(|t| t.id == id).unwrap().opened;
    assert_eq!(opened("tlsrec"), 4);
    assert_eq!(opened("tlsrecdisorder"), 4);
    for id in ["disorder", "disordersni", "splitsni", "split"] {
        assert_eq!(opened(id), 0, "{id}: TCP bolmesi birlestiren DPI'yi gecemez");
    }
    assert_eq!(out.best().unwrap().id, "tlsrec", "esitlikte liste sirasi");
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dpi-macservice-ara-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn controller(name: &str, fetch: Arc<Fetch>) -> Arc<Controller> {
    let cfg = AppConfig { language: Language::Turkish, proxy_method: "split".into(), ..AppConfig::default() }.sanitized();
    Controller::with_fetch(cfg, Box::new(Untouched), temp_dir(name), 0, fetch)
}

#[test]
fn no_blocking_keeps_the_method_and_no_internet_is_an_error() {
    let open = controller("acik", Arc::new(|_: SocketAddr, _: &str| true));
    let Response::MethodSearch { best, blocked, results } = open.handle(Request::FindMethod, None) else { panic!() };
    assert!(!blocked && best.is_none());
    assert_eq!(results.len(), proxy::presets::METHODS.len());
    assert_eq!(open.config().proxy_method, "split", "engel yoksa yontem degismez");

    let offline = controller("yok", Arc::new(|_: SocketAddr, _: &str| false));
    let r = offline.handle(Request::FindMethod, None);
    assert!(matches!(&r, Response::Error { message } if message.contains("İnternet")), "{r:?}");
}

#[test]
fn blocked_line_saves_the_best_method() {
    // Yalnizca "disorder" proxy'si uzerinden acilan siteler: hangi proxy'nin hangi yontem oldugu
    // bilinmedigi icin burada her yontem esit; liste sirasindaki ilk yontem secilir.
    let ctrl = controller("kayit", Arc::new(|_: SocketAddr, url: &str| url.contains("example") || url.contains("discord")));
    let Response::MethodSearch { best, blocked, .. } = ctrl.handle(Request::FindMethod, None) else { panic!() };
    assert!(blocked);
    assert_eq!(best.as_deref(), Some("tlsrec"));
    assert_eq!(ctrl.config().proxy_method, "tlsrec", "secilen yontem kaydedilir");
}
