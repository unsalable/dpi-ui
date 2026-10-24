//! Gelistirme araci: proxy motorunu tek basina calistirir (Windows'ta canli deneme icin).
//!
//!   dpi-proxy [--port 47813] [--method disorder|none|...] [--dns <url>|system] [--bootstrap ip,ip] [--verbose]
//!   dpi-proxy --methods
//!
//! Baska bir terminalden: `curl.exe -x http://127.0.0.1:47813 https://discord.com/`

use std::net::IpAddr;
use std::sync::Arc;

use proxy::{Desync, DohResolver, Event, Options, Proxy, Resolve, SystemResolver, presets};

const DEFAULT_DOH: &str = "https://cloudflare-dns.com/dns-query";
const DEFAULT_BOOTSTRAP: &str = "1.1.1.1,1.0.0.1";

fn main() {
    let mut port = proxy::DEFAULT_PORT;
    let mut method = presets::DEFAULT_ID.to_owned();
    let mut dns = DEFAULT_DOH.to_owned();
    let mut bootstrap = DEFAULT_BOOTSTRAP.to_owned();
    let mut verbose = false;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| fail(&format!("{a} için değer yok")));
        match a.as_str() {
            "--port" => port = value().parse().unwrap_or_else(|_| fail("geçersiz port")),
            "--method" => method = value(),
            "--dns" => dns = value(),
            "--bootstrap" => bootstrap = value(),
            "--verbose" => verbose = true,
            "--methods" => {
                for m in presets::METHODS {
                    println!("{:<16} {} — {}", m.id, m.name, m.description);
                }
                return;
            }
            _ => fail(&format!("bilinmeyen seçenek: {a}")),
        }
    }

    let desync = if method == "none" {
        Desync::default()
    } else {
        presets::method(&method).unwrap_or_else(|| fail(&format!("bilinmeyen yöntem: {method} (--methods)"))).desync()
    };
    let resolver: Arc<dyn Resolve> = if dns == "system" {
        Arc::new(SystemResolver)
    } else {
        let ips: Vec<IpAddr> = bootstrap.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        Arc::new(DohResolver::new(&dns, &ips, false).unwrap_or_else(|e| fail(&e)))
    };

    let mut opts = Options::new(port, desync, resolver);
    if verbose {
        opts.events = Some(Arc::new(|e: Event| match e {
            Event::Opened { host, port, addr } => eprintln!("bağlandı  {host}:{port} -> {addr}"),
            Event::Desynced { host, segments } => eprintln!("parçalı   {host} ({segments} parça)"),
            Event::Failed { host, port, error } => eprintln!("HATA      {host}:{port}: {error}"),
        }));
    }
    let proxy = Proxy::start(opts).unwrap_or_else(|e| fail(&format!("dinlenemedi: {e}")));
    println!("dinleniyor {} · yöntem: {method} · DNS: {dns}", proxy.local_addr());
    loop {
        std::thread::park();
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("dpi-proxy: {msg}");
    std::process::exit(2)
}
