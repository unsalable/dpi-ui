//! "En iyi yontemi bul": yontemsiz (yalnizca sifreli DNS) ve her hazir yontemle ayri gecici
//! proxy'ler acar, engelli bilinen siteleri bu proxy'ler uzerinden ister. Hepsi paralel
//! calisir (~7 sn). Engelli siteleri en cok acan ilk yontem secilir; liste sirasi oneri sirasi.
//! Windows'ta yapilan canli olcumun (perf/proxy-live-test.ps1) uygulama ici hali.

use std::net::SocketAddr;
use std::sync::Arc;

use proxy::{Desync, Options, Proxy, Resolve, presets};
use shared::ipc::MethodTrial;

/// Turkiye'de engelli bilinen siteler (Ekim 2026).
pub const TARGETS: &[&str] = &["https://discord.com/", "https://gateway.discord.gg/", "https://www.roblox.com/", "https://www.wattpad.com/"];
/// Engelsiz site: buna da ulasilamiyorsa internet yok demektir.
pub const CONTROL: &str = "https://example.com/";

/// Bir istegi proxy uzerinden yapar; sunucudan herhangi bir HTTP yaniti geldiyse true.
pub type Fetch = dyn Fn(SocketAddr, &str) -> bool + Send + Sync;

#[derive(Debug)]
pub struct Outcome {
    /// Yontemsiz denemede acilan engelli site sayisi.
    pub baseline: u32,
    /// Engelsiz siteye ulasildi mi?
    pub online: bool,
    pub trials: Vec<MethodTrial>,
}

impl Outcome {
    pub fn blocked(&self) -> bool {
        self.baseline < TARGETS.len() as u32
    }

    /// Engelli siteleri en cok acan ilk yontem (hicbiri acmadiysa yok).
    pub fn best(&self) -> Option<&MethodTrial> {
        let max = self.trials.iter().map(|t| t.opened).max().filter(|&m| m > 0)?;
        self.trials.iter().find(|t| t.opened == max)
    }
}

/// Bir yontemle (ya da yontemsiz) tum hedefleri dener; acilan hedef sayisi ve kontrol sonucu.
fn try_desync(desync: Desync, resolver: Arc<dyn Resolve>, fetch: Arc<Fetch>, control: bool) -> (u32, bool) {
    let Ok(proxy) = Proxy::start(Options::new(0, desync, resolver)) else { return (0, false) };
    let addr = proxy.local_addr();
    let urls: Vec<&'static str> = TARGETS.iter().copied().chain(control.then_some(CONTROL)).collect();
    let handles: Vec<_> = urls
        .into_iter()
        .map(|u| {
            let fetch = fetch.clone();
            std::thread::spawn(move || (u, fetch(addr, u)))
        })
        .collect();
    let mut opened = 0;
    let mut online = false;
    for h in handles {
        if let Ok((u, ok)) = h.join() {
            if u == CONTROL {
                online = ok;
            } else if ok {
                opened += 1;
            }
        }
    }
    proxy.stop();
    (opened, online)
}

pub fn run(resolver: Arc<dyn Resolve>, fetch: Arc<Fetch>, en: bool) -> Outcome {
    let base = {
        let (r, f) = (resolver.clone(), fetch.clone());
        std::thread::spawn(move || try_desync(Desync::default(), r, f, true))
    };
    let methods: Vec<_> = presets::METHODS
        .iter()
        .map(|m| {
            let (r, f) = (resolver.clone(), fetch.clone());
            (m, std::thread::spawn(move || try_desync(m.desync(), r, f, false).0))
        })
        .collect();
    let (baseline, online) = base.join().unwrap_or((0, false));
    let total = TARGETS.len() as u32;
    let trials = methods
        .into_iter()
        .map(|(m, h)| MethodTrial { id: m.id.into(), name: m.label(en).into(), opened: h.join().unwrap_or(0), total })
        .collect();
    Outcome { baseline, online, trials }
}

/// Gercek istek: curl proxy uzerinden (macOS'ta her zaman var; Windows 10+ da curl.exe tasir).
pub fn curl(proxy: SocketAddr, url: &str) -> bool {
    let (exe, null) = if cfg!(windows) { ("curl.exe", "NUL") } else { ("/usr/bin/curl", "/dev/null") };
    std::process::Command::new(exe)
        .args(["-s", "-o", null, "-x", &format!("http://{proxy}"), "--connect-timeout", "5", "--max-time", "7", "-w", "%{http_code}", url])
        .output()
        .is_ok_and(|o| {
            let code = String::from_utf8_lossy(&o.stdout);
            !code.trim().is_empty() && code.trim() != "000"
        })
}
