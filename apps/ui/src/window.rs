//! Pencere sureci: Slint arayuzu + servis durumunun ekrana yansitilmasi.
//! Tum ayar degisiklikleri servise gonderilir; ekran servisten gelen olaylarla guncellenir
//! (tek dogruluk kaynagi servis).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use engine::presets;
use shared::config::{AppConfig, CustomDns, CustomDnsKind, DnsMode};
use shared::ipc::{EngineState, EngineStatus, Event, LearnedHost, Request, Response, StateSnapshot};
use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::client::{Client, Incoming};
use crate::prefs::{self, Prefs};
use crate::winsys;
use crate::{AppState, AppWindow, BenchItem, CategoryItem, Choice, HostItem, PowerState, Theme};

struct Ctx {
    app: slint::Weak<AppWindow>,
    client: RefCell<Option<Client>>,
    config: RefCell<Option<AppConfig>>,
    engine: RefCell<EngineStatus>,
    conflicts: RefCell<Vec<shared::ipc::Conflict>>,
    learned: RefCell<Vec<LearnedHost>>,
    connected: Cell<bool>,
    prefs: RefCell<Prefs>,
    toast_serial: Cell<i32>,
    method_ids: RefCell<Vec<String>>,
    dns_ids: RefCell<Vec<String>>,
    home_visible: Cell<bool>,
    /// GDPI3_TEST: gercek oturum acilisi kaydina, tepsiye ve v1 gocune dokunma.
    test_mode: bool,
}

thread_local! {
    static CTX: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

fn with_ctx(f: impl FnOnce(&Rc<Ctx>)) {
    CTX.with(|c| {
        if let Some(ctx) = c.borrow().as_ref() {
            f(ctx);
        }
    });
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn s(v: impl Into<SharedString>) -> SharedString {
    v.into()
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// IPC is parcacigindan arayuze bildirim gonderir.
fn post_toast(message: String, error: bool) {
    let _ = slint::invoke_from_event_loop(move || with_ctx(|c| c.toast(&message, error)));
}

impl Ctx {
    fn ui(&self) -> Option<AppWindow> {
        self.app.upgrade()
    }

    fn client(&self) -> Option<Client> {
        self.client.borrow().clone()
    }

    fn toast(&self, message: &str, error: bool) {
        let Some(app) = self.ui() else { return };
        let st = app.global::<AppState>();
        st.set_toast(s(message));
        st.set_toast_error(error);
        self.toast_serial.set(self.toast_serial.get() + 1);
        st.set_toast_serial(self.toast_serial.get());
    }

    /// Ayari kopyalayip degistirir ve servise gonderir. Ekran, servisin onayladigi
    /// yeni ayar olay olarak gelince guncellenir.
    fn edit(&self, f: impl FnOnce(&mut AppConfig)) {
        let (Some(mut cfg), Some(client)) = (self.config.borrow().clone(), self.client()) else {
            self.toast("Motor hizmetine bağlı değil.", true);
            return;
        };
        f(&mut cfg);
        client.request(Request::SetConfig { config: Box::new(cfg) }, |r| {
            if let Response::Error { message } = r {
                post_toast(message, true);
            }
        });
    }

    fn save_prefs(&self) {
        prefs::save(&self.prefs.borrow());
    }

    // ------------------------------------------------------------ durum yansitma

    fn on_incoming(&self, inc: Incoming) {
        match inc {
            Incoming::Connected(state) => self.apply_snapshot(*state),
            Incoming::Disconnected => {
                self.connected.set(false);
                *self.config.borrow_mut() = None;
                self.apply_engine();
            }
            Incoming::Event(ev) => match ev {
                Event::Engine { status } => {
                    *self.engine.borrow_mut() = status;
                    self.apply_engine();
                }
                Event::Config { config } => self.apply_config(*config),
                Event::Learned { host } => {
                    self.toast(&format!("{} açılmıyordu; artık otomatik atlatılıyor.", host.host), false);
                    self.learned.borrow_mut().push(host);
                    self.apply_learned();
                }
                Event::Conflicts { conflicts } => {
                    *self.conflicts.borrow_mut() = conflicts;
                    self.apply_engine();
                }
            },
        }
    }

    fn apply_snapshot(&self, state: StateSnapshot) {
        self.connected.set(true);
        *self.engine.borrow_mut() = state.engine;
        *self.conflicts.borrow_mut() = state.conflicts;
        *self.learned.borrow_mut() = state.learned;
        if let Some(app) = self.ui() {
            app.global::<AppState>().set_version(s(state.service_version.as_str()));
        }
        let fresh = state.config == AppConfig::default().sanitized();
        self.apply_config(state.config);
        self.apply_learned();
        self.apply_engine();
        self.first_run_tasks(fresh);
    }

    fn apply_engine(&self) {
        let Some(app) = self.ui() else { return };
        let st = app.global::<AppState>();
        let e = self.engine.borrow();
        st.set_service_missing(!self.connected.get());
        if !self.connected.get() {
            st.set_power(PowerState::Off);
            st.set_status_title(s("Motor hizmeti kapalı"));
            st.set_status_sub(s("Başlatmak için küreye dokun"));
            st.set_banner(s(""));
            st.set_stats_line(s(""));
            return;
        }
        let (power, title, sub) = match e.state {
            EngineState::Off => (PowerState::Off, "Kapalı".to_owned(), "Açmak için küreye dokun".to_owned()),
            EngineState::Starting => (PowerState::Connecting, "Bağlanıyor…".to_owned(), "Motor başlatılıyor".to_owned()),
            EngineState::On => (PowerState::On, "Korumalı".to_owned(), uptime_text(e.since_ms)),
            EngineState::Paused => (
                PowerState::Off,
                "Duraklatıldı".to_owned(),
                e.message.clone().unwrap_or_else(|| "Bir uygulama için geçici olarak durduruldu".into()),
            ),
            EngineState::Error => (
                PowerState::Error,
                "Bağlanamadı".to_owned(),
                e.message.clone().unwrap_or_else(|| "Bilinmeyen hata".into()),
            ),
        };
        st.set_power(power);
        st.set_status_title(s(title));
        st.set_status_sub(s(sub));
        st.set_method_name(s(e.method_name.as_str()));
        st.set_dns_name(s(e.dns_name.as_str()));

        let conflicts = self.conflicts.borrow();
        let banner = if !conflicts.is_empty() {
            let names: Vec<&str> = conflicts.iter().map(|c| c.description.as_str()).collect();
            format!("{}. Aynı anda iki DPI aracı bağlantıları bozabilir; diğerini kapat.", names.join(", "))
        } else if e.state == EngineState::On {
            e.message.clone().unwrap_or_default()
        } else {
            String::new()
        };
        st.set_banner(s(banner));
        if e.state != EngineState::On {
            st.set_stats_line(s(""));
        }
    }

    fn apply_config(&self, cfg: AppConfig) {
        let Some(app) = self.ui() else { return };
        let st = app.global::<AppState>();

        // Saglayicilar.
        let isps: Vec<Choice> = presets::ISPS
            .iter()
            .map(|i| Choice { id: s(i.id), title: s(i.name), subtitle: s(i.description) })
            .collect();
        let isp_index = presets::ISPS.iter().position(|i| i.id.eq_ignore_ascii_case(&cfg.isp)).unwrap_or(0);
        st.set_isp_names(model(isps.iter().map(|c| c.title.clone()).collect()));
        st.set_isps(model(isps));
        st.set_isp_index(isp_index as i32);

        // Yontemler: secili saglayicinin onerileri basta, sonra digerleri, sonra ozeller.
        let isp = &presets::ISPS[isp_index];
        let mut ids: Vec<String> = Vec::new();
        let mut items: Vec<Choice> = Vec::new();
        for (n, id) in isp.methods.iter().enumerate() {
            if let Some(m) = presets::method(id) {
                let tag = if n == 0 { "Önerilen · " } else { "" };
                items.push(Choice { id: s(m.id), title: s(format!("{tag}{}", m.name)), subtitle: s(m.config().short_summary()) });
                ids.push(m.id.into());
            }
        }
        for m in presets::ALL.iter().filter(|m| !isp.methods.contains(&m.id)) {
            items.push(Choice { id: s(m.id), title: s(m.name), subtitle: s(m.config().short_summary()) });
            ids.push(m.id.into());
        }
        for c in &cfg.custom_methods {
            items.push(Choice { id: s(c.id.as_str()), title: s(format!("Özel · {}", c.name)), subtitle: s(c.config.short_summary()) });
            ids.push(c.id.clone());
        }
        let method_index = ids.iter().position(|i| i.eq_ignore_ascii_case(&cfg.method)).unwrap_or(0);
        st.set_methods(model(items));
        st.set_method_index(method_index as i32);
        *self.method_ids.borrow_mut() = ids;

        // DNS.
        st.set_dns_mode(match cfg.dns.mode {
            DnsMode::Off => 0,
            DnsMode::Encrypted => 1,
            DnsMode::Redirect => 2,
        });
        let mut dns_ids = Vec::new();
        let mut providers = Vec::new();
        for p in shared::dns::PRESETS {
            let fits = match cfg.dns.mode {
                DnsMode::Encrypted => p.supports_doh(),
                DnsMode::Redirect => p.supports_redirect(),
                DnsMode::Off => false,
            };
            if fits {
                let detail = match cfg.dns.mode {
                    DnsMode::Redirect => {
                        let (a, port) = p.udp_v4.unwrap_or(("", 53));
                        format!("{} · {a}:{port}", p.description)
                    }
                    _ => format!("{} · {}", p.description, if p.doh_url.starts_with("tls://") { "DoT" } else { "DoH" }),
                };
                providers.push(Choice { id: s(p.id), title: s(p.name), subtitle: s(detail) });
                dns_ids.push(p.id.to_owned());
            }
        }
        for c in &cfg.dns.custom {
            let fits = matches!((cfg.dns.mode, c.kind), (DnsMode::Encrypted, CustomDnsKind::Doh) | (DnsMode::Redirect, CustomDnsKind::Udp));
            if fits {
                providers.push(Choice { id: s(c.id.as_str()), title: s(format!("Özel · {}", c.name)), subtitle: s(custom_dns_address(c)) });
                dns_ids.push(c.id.clone());
            }
        }
        let dns_index = dns_ids.iter().position(|i| i.eq_ignore_ascii_case(&cfg.dns.provider)).map_or(-1, |i| i as i32);
        st.set_dns_providers(model(providers));
        st.set_dns_index(dns_index);
        *self.dns_ids.borrow_mut() = dns_ids;
        st.set_custom_dns(model(
            cfg.dns
                .custom
                .iter()
                .map(|c| HostItem { key: s(c.id.as_str()), name: s(c.name.as_str()), note: s(custom_dns_address(c)) })
                .collect(),
        ));
        st.set_dns_fallback(cfg.dns.fallback_to_system);
        st.set_dns_skip_vpn(cfg.dns.skip_on_vpn);

        // Siteler.
        st.set_scope_mode(if cfg.scope.mode == engine::ScopeMode::Smart { 0 } else { 1 });
        st.set_learning(cfg.scope.learning);
        st.set_categories(model(
            engine::scope::BUILTIN_TARGETS
                .iter()
                .map(|(id, _)| CategoryItem {
                    id: s(*id),
                    title: s(category_title(id)),
                    checked: cfg.scope.categories.iter().any(|c| c.eq_ignore_ascii_case(id)),
                })
                .collect(),
        ));
        st.set_targets(model(host_items(&cfg.scope.targets)));
        st.set_excluded(model(host_items(&cfg.scope.excluded)));

        // Genel.
        st.set_start_with_windows(cfg.start_with_windows);
        st.set_auto_connect(cfg.auto_connect);
        st.set_method_name(s(cfg.active_method().name));
        st.set_dns_name(s(cfg.active_dns().display_name()));

        *self.config.borrow_mut() = Some(cfg);
    }

    fn apply_learned(&self) {
        let Some(app) = self.ui() else { return };
        let items: Vec<HostItem> = self
            .learned
            .borrow()
            .iter()
            .rev()
            .map(|h| HostItem { key: s(h.host.as_str()), name: s(h.host.as_str()), note: s(h.reason.as_str()) })
            .collect();
        app.global::<AppState>().set_learned(model(items));
    }

    fn apply_prefs(&self) {
        let Some(app) = self.ui() else { return };
        let p = self.prefs.borrow();
        let st = app.global::<AppState>();
        st.set_theme_index(p.theme as i32);
        st.set_close_to_tray(p.close_to_tray);
        st.set_tray_at_login(p.tray_at_login);
        let dark = match p.theme {
            1 => true,
            2 => false,
            _ => !winsys::system_prefers_light(),
        };
        app.global::<Theme>().set_dark(dark);
    }

    // ------------------------------------------------------------ ilk calistirma

    fn first_run_tasks(&self, fresh_config: bool) {
        if self.test_mode {
            return;
        }
        let mut p = self.prefs.borrow_mut();
        if !p.autorun_initialized {
            winsys::set_autorun(p.tray_at_login);
            p.autorun_initialized = true;
        }
        if !p.v1_checked {
            p.v1_checked = true;
            if fresh_config {
                if let Some(text) = shared::paths::v1_settings_file().and_then(|f| std::fs::read_to_string(f).ok()) {
                    if let Some(m) = shared::migrate::from_v1(&text) {
                        if let Some(dark) = m.ui.dark_mode {
                            p.theme = if dark { 1 } else { 2 };
                        }
                        if let Some(client) = self.client() {
                            let summary = if m.summary.is_empty() { "varsayılanlar".to_owned() } else { m.summary.join(", ") };
                            client.request(Request::SetConfig { config: Box::new(m.config) }, move |r| match r {
                                Response::Ok => post_toast(format!("Eski sürüm ayarların aktarıldı: {summary}."), false),
                                Response::Error { message } => post_toast(format!("Eski ayarlar aktarılamadı: {message}"), true),
                                _ => {}
                            });
                        }
                    }
                }
            }
        }
        drop(p);
        self.save_prefs();
        self.apply_prefs();
    }

    // ------------------------------------------------------------ eylemler

    fn power_toggle(&self) {
        let Some(client) = self.client() else { return };
        let state = self.engine.borrow().state;
        let request = if matches!(state, EngineState::On | EngineState::Starting) { Request::Disconnect } else { Request::Connect };
        client.request(request, |r| {
            if let Response::Error { message } = r {
                post_toast(message, true);
            }
        });
    }

    fn add_custom_dns(&self, name: &str, address: &str, bootstrap: &str) -> String {
        let entry = match parse_custom_dns(name, address, bootstrap) {
            Ok(e) => e,
            Err(e) => return e,
        };
        let taken: Vec<String> = self.config.borrow().as_ref().map(|c| c.dns.custom.iter().map(|d| d.id.clone()).collect()).unwrap_or_default();
        let id = new_custom_id(&taken);
        let mode = if entry.kind == CustomDnsKind::Doh { DnsMode::Encrypted } else { DnsMode::Redirect };
        self.edit(move |c| {
            c.dns.custom.push(CustomDns { id: id.clone(), ..entry });
            c.dns.provider = id;
            c.dns.mode = mode;
        });
        String::new()
    }

    fn run_bench(&self) {
        let (Some(app), Some(client)) = (self.ui(), self.client()) else { return };
        app.global::<AppState>().set_bench_running(true);
        client.request(Request::BenchDns, |r| {
            let _ = slint::invoke_from_event_loop(move || {
                with_ctx(|c| {
                    let Some(app) = c.ui() else { return };
                    let st = app.global::<AppState>();
                    st.set_bench_running(false);
                    match r {
                        Response::DnsBench { results } => {
                            let best = results.iter().filter_map(|r| (r.ok == r.total).then_some(r.median_ms).flatten()).min().unwrap_or(1).max(1);
                            let worst = results.iter().filter_map(|r| r.median_ms).max().unwrap_or(best).max(best);
                            let items: Vec<BenchItem> = results
                                .iter()
                                .map(|r| {
                                    let ok = r.ok == r.total && r.median_ms.is_some();
                                    let ms = r.median_ms.unwrap_or(0);
                                    let ratio = if worst == best { 1.0 } else { 1.0 - 0.8 * (ms.saturating_sub(best)) as f32 / (worst - best) as f32 };
                                    BenchItem {
                                        id: s(r.id.as_str()),
                                        name: s(r.name.as_str()),
                                        ms: ms as i32,
                                        ok,
                                        ratio,
                                        note: s(if ok { format!("{ms} ms") } else { "hata".into() }),
                                    }
                                })
                                .collect();
                            st.set_bench(model(items));
                        }
                        Response::Error { message } => c.toast(&message, true),
                        _ => {}
                    }
                });
            });
        });
    }

    fn add_host(&self, host: &str, excluded: bool) -> String {
        let Some(name) = engine::scope::normalize(host) else {
            return "Geçerli bir alan adı gir (örn. site.com).".into();
        };
        let exists = self.config.borrow().as_ref().is_some_and(|c| {
            let list = if excluded { &c.scope.excluded } else { &c.scope.targets };
            list.iter().any(|h| h == &name)
        });
        if exists {
            return "Zaten listede.".into();
        }
        self.edit(|c| {
            if excluded { c.scope.excluded.push(name) } else { c.scope.targets.push(name) }
        });
        String::new()
    }

    fn refresh_state(&self) {
        let Some(client) = self.client() else { return };
        client.request(Request::GetState, |r| {
            if let Response::State { state } = r {
                let _ = slint::invoke_from_event_loop(move || {
                    with_ctx(|c| {
                        *c.learned.borrow_mut() = state.learned;
                        c.apply_learned();
                    })
                });
            }
        });
    }

    fn poll_stats(&self) {
        if !self.home_visible.get() || self.engine.borrow().state != EngineState::On {
            return;
        }
        let Some(client) = self.client() else { return };
        client.request(Request::GetStats, |r| {
            if let Response::Stats { stats } = r {
                let _ = slint::invoke_from_event_loop(move || {
                    with_ctx(|c| {
                        let Some(app) = c.ui() else { return };
                        let line = if stats.bypassed == 0 && stats.dns_queries == 0 {
                            String::new()
                        } else if stats.dns_queries > 0 {
                            format!("{} bağlantı atlatıldı · {} şifreli DNS sorgusu", count(stats.bypassed), count(stats.dns_queries))
                        } else {
                            format!("{} bağlantı atlatıldı", count(stats.bypassed))
                        };
                        app.global::<AppState>().set_stats_line(s(line));
                    })
                });
            }
        });
    }
}

fn count(n: u64) -> String {
    if n >= 10_000 { format!("{:.1}B", n as f64 / 1000.0).replace('.', ",") } else { n.to_string() }
}

fn uptime_text(since: Option<u64>) -> String {
    let Some(since) = since else { return "Bağlantı korunuyor".into() };
    let mins = now_ms().saturating_sub(since) / 60_000;
    match mins {
        0 => "Az önce bağlandı".into(),
        1..=59 => format!("{mins} dakikadır korumalı"),
        _ => format!("{} sa {} dk'dır korumalı", mins / 60, mins % 60),
    }
}

fn category_title(id: &str) -> &'static str {
    match id {
        "discord" => "Discord",
        "roblox" => "Roblox",
        "wattpad" => "Wattpad",
        "vpn" => "VPN sağlayıcılarının siteleri",
        _ => "Diğer",
    }
}

fn host_items(list: &[String]) -> Vec<HostItem> {
    list.iter().map(|h| HostItem { key: s(h.as_str()), name: s(h.as_str()), note: s("") }).collect()
}

fn custom_dns_address(c: &CustomDns) -> String {
    match c.kind {
        CustomDnsKind::Doh => c.doh_url.clone(),
        CustomDnsKind::Udp => {
            if !c.v4.is_empty() {
                format!("{}:{}", c.v4, c.v4_port)
            } else {
                format!("[{}]:{}", c.v6, c.v6_port)
            }
        }
    }
}

fn new_custom_id(taken: &[String]) -> String {
    if !taken.iter().any(|t| t.eq_ignore_ascii_case("custom")) {
        return "custom".into();
    }
    let seed = now_ms();
    (0..1000u64)
        .map(|i| format!("custom:{:08x}", (seed.wrapping_mul(2_654_435_761).wrapping_add(i)) & 0xffff_ffff))
        .find(|id| !taken.iter().any(|t| t.eq_ignore_ascii_case(id)))
        .unwrap_or_else(|| "custom:ffffffff".into())
}

/// Kullanicinin yazdigi adresi ozel DNS girisine cevirir.
fn parse_custom_dns(name: &str, address: &str, bootstrap: &str) -> Result<CustomDns, String> {
    let a = address.trim();
    if a.is_empty() {
        return Err("Bir adres gir.".into());
    }
    let name = if name.trim().is_empty() { "Özel DNS".to_owned() } else { name.trim().to_owned() };
    let mut d = CustomDns { name, ..CustomDns::default() };
    if a.starts_with("https://") || a.starts_with("tls://") {
        d.kind = CustomDnsKind::Doh;
        d.doh_url = a.to_owned();
        d.bootstrap = bootstrap.trim().to_owned();
    } else if let Ok(sa) = a.parse::<std::net::SocketAddr>() {
        match sa {
            std::net::SocketAddr::V4(v) => {
                d.v4 = v.ip().to_string();
                d.v4_port = v.port();
            }
            std::net::SocketAddr::V6(v) => {
                d.v6 = v.ip().to_string();
                d.v6_port = v.port();
            }
        }
    } else if let Ok(ip) = a.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V4(v) => {
                d.v4 = v.to_string();
                d.v4_port = 53;
            }
            std::net::IpAddr::V6(v) => {
                d.v6 = v.to_string();
                d.v6_port = 53;
            }
        }
    } else {
        return Err("Adres anlaşılamadı. IP (1.2.3.4), IP:port ya da https://… / tls://… gir.".into());
    }
    d.validate()?;
    Ok(d)
}

// ================================================================ pencere

pub fn run(show_event: Option<windows_sys::Win32::Foundation::HANDLE>) -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;
    let ctx = Rc::new(Ctx {
        app: app.as_weak(),
        client: RefCell::new(None),
        config: RefCell::new(None),
        engine: RefCell::new(EngineStatus::default()),
        conflicts: RefCell::new(Vec::new()),
        learned: RefCell::new(Vec::new()),
        connected: Cell::new(false),
        prefs: RefCell::new(prefs::load()),
        toast_serial: Cell::new(0),
        method_ids: RefCell::new(Vec::new()),
        dns_ids: RefCell::new(Vec::new()),
        home_visible: Cell::new(true),
        test_mode: std::env::var_os("GDPI3_TEST").is_some(),
    });
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    ctx.apply_prefs();
    ctx.apply_engine();
    app.global::<AppState>().set_version(s(shared::VERSION));

    bind_callbacks(&app);

    // Servis baglantisi: olaylar IPC is parcacigindan arayuz dongusune aktarilir.
    let client = Client::start("pencere", |inc| {
        let _ = slint::invoke_from_event_loop(move || with_ctx(|c| c.on_incoming(inc)));
    });
    *ctx.client.borrow_mut() = Some(client);

    // Ikinci kez acilinca var olan pencere one gelir.
    if let Some(ev) = show_event {
        let ev = ev as usize;
        std::thread::Builder::new()
            .name("tek-ornek".into())
            .stack_size(64 * 1024)
            .spawn(move || loop {
                use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};
                unsafe { WaitForSingleObject(ev as _, INFINITE) };
                let _ = slint::invoke_from_event_loop(|| {
                    with_ctx(|c| {
                        if let Some(app) = c.ui() {
                            let _ = app.show();
                            app.window().with_winit_window(|w| {
                                w.set_minimized(false);
                                w.focus_window();
                            });
                        }
                    })
                });
            })
            .ok();
    }

    // Kapatinca tepside kalinacaksa tepsi sureci simdiden acilsin.
    if ctx.prefs.borrow().close_to_tray && !ctx.test_mode {
        spawn_tray();
    }

    // Test/ekran goruntusu: GDPI3_PAGE="derinlik:altsayfa", GDPI3_LIGHT=1.
    if ctx.test_mode {
        if let Some((d, sub)) = std::env::var("GDPI3_PAGE").ok().and_then(|v| {
            let (a, b) = v.split_once(':')?;
            Some((a.parse::<i32>().ok()?, b.parse::<i32>().ok()?))
        }) {
            app.set_depth(d);
            app.set_sub(sub);
            ctx.home_visible.set(d == 0);
        }
        if std::env::var_os("GDPI3_LIGHT").is_some() {
            app.global::<Theme>().set_dark(false);
        }
    }

    // Sure metni (yalnizca bagliyken) ve sayaclar (yalnizca ana sayfa gorunurken).
    let uptime = Timer::default();
    uptime.start(TimerMode::Repeated, Duration::from_secs(20), || {
        with_ctx(|c| {
            if c.engine.borrow().state == EngineState::On {
                c.apply_engine();
            }
        })
    });
    let stats = Timer::default();
    stats.start(TimerMode::Repeated, Duration::from_secs(3), || with_ctx(|c| c.poll_stats()));

    app.show()?;
    style_window(&app);
    slint::run_event_loop()?;
    CTX.with(|c| *c.borrow_mut() = None);
    Ok(())
}

fn spawn_tray() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--tray").spawn();
    }
}

fn bind_callbacks(app: &AppWindow) {
    app.on_drag_window({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                app.window().with_winit_window(|w| {
                    let _ = w.drag_window();
                });
            }
        }
    });
    app.on_minimize({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                app.window().set_minimized(true);
            }
        }
    });
    app.on_close_window(|| {
        with_ctx(|c| {
            if c.prefs.borrow().close_to_tray && !c.test_mode {
                spawn_tray();
            }
        });
        let _ = slint::quit_event_loop();
    });
    app.on_page_changed(|depth, _| with_ctx(|c| c.home_visible.set(depth == 0)));

    let st = app.global::<AppState>();
    st.on_power_toggle(|| with_ctx(|c| c.power_toggle()));
    st.on_start_service(|| {
        with_ctx(|c| match winsys::start_service() {
            Ok(()) => c.toast("Motor hizmeti başlatılıyor…", false),
            Err(e) => c.toast(&e, true),
        })
    });
    st.on_select_isp(|i| {
        with_ctx(|c| {
            let Some(isp) = presets::ISPS.get(i as usize) else { return };
            let rec = isp.recommended();
            c.edit(|cfg| {
                cfg.isp = isp.id.into();
                cfg.method = rec.id.into();
            });
            if isp.id != "general" {
                c.toast(&format!("{} için önerilen yöntem seçildi: {}.", isp.name, rec.name), false);
            }
        })
    });
    st.on_select_method(|i| {
        with_ctx(|c| {
            let Some(id) = c.method_ids.borrow().get(i as usize).cloned() else { return };
            c.edit(|cfg| cfg.method = id);
        })
    });
    st.on_set_dns_mode(|i| {
        with_ctx(|c| {
            c.edit(|cfg| {
                cfg.dns.mode = match i {
                    0 => DnsMode::Off,
                    2 => DnsMode::Redirect,
                    _ => DnsMode::Encrypted,
                };
                // Saglayici bu modu desteklemiyorsa uygun bir varsayilana gec.
                if cfg.validate().is_err() {
                    cfg.dns.provider = if cfg.dns.mode == DnsMode::Redirect { "yandex".into() } else { "cloudflare".into() };
                }
            })
        })
    });
    st.on_select_dns(|i| {
        with_ctx(|c| {
            let Some(id) = c.dns_ids.borrow().get(i as usize).cloned() else { return };
            c.edit(|cfg| cfg.dns.provider = id);
        })
    });
    st.on_set_dns_fallback(|v| with_ctx(|c| c.edit(|cfg| cfg.dns.fallback_to_system = v)));
    st.on_set_dns_skip_vpn(|v| with_ctx(|c| c.edit(|cfg| cfg.dns.skip_on_vpn = v)));
    st.on_add_custom_dns(|name, addr, boot| {
        let mut out = String::new();
        with_ctx(|c| out = c.add_custom_dns(&name, &addr, &boot));
        s(out)
    });
    st.on_remove_custom_dns(|id| {
        let id = id.to_string();
        with_ctx(|c| {
            c.edit(|cfg| {
                cfg.dns.custom.retain(|d| d.id != id);
                if cfg.dns.provider == id {
                    cfg.dns.provider = if cfg.dns.mode == DnsMode::Redirect { "yandex".into() } else { "cloudflare".into() };
                }
            })
        })
    });
    st.on_run_bench(|| with_ctx(|c| c.run_bench()));
    st.on_use_bench(|id| {
        let id = id.to_string();
        with_ctx(|c| {
            c.edit(|cfg| {
                cfg.dns.mode = DnsMode::Encrypted;
                cfg.dns.provider = id;
            });
            c.toast("DNS değiştirildi.", false);
        })
    });
    st.on_set_scope_mode(|i| {
        with_ctx(|c| c.edit(|cfg| cfg.scope.mode = if i == 1 { engine::ScopeMode::All } else { engine::ScopeMode::Smart }))
    });
    st.on_set_learning(|v| with_ctx(|c| c.edit(|cfg| cfg.scope.learning = v)));
    st.on_toggle_category(|id, on| {
        let id = id.to_string();
        with_ctx(|c| {
            c.edit(|cfg| {
                cfg.scope.categories.retain(|x| !x.eq_ignore_ascii_case(&id));
                if on {
                    cfg.scope.categories.push(id);
                }
            })
        })
    });
    st.on_add_target(|h| {
        let mut out = String::new();
        with_ctx(|c| out = c.add_host(&h, false));
        s(out)
    });
    st.on_remove_target(|h| {
        let h = h.to_string();
        with_ctx(|c| c.edit(|cfg| cfg.scope.targets.retain(|x| x != &h)))
    });
    st.on_add_excluded(|h| {
        let mut out = String::new();
        with_ctx(|c| out = c.add_host(&h, true));
        s(out)
    });
    st.on_remove_excluded(|h| {
        let h = h.to_string();
        with_ctx(|c| c.edit(|cfg| cfg.scope.excluded.retain(|x| x != &h)))
    });
    st.on_forget_learned(|h| {
        let host = h.to_string();
        with_ctx(|c| {
            if let Some(client) = c.client() {
                client.request(Request::ForgetLearned { host }, |_| {});
            }
            c.refresh_state();
        })
    });
    st.on_clear_learned(|| {
        with_ctx(|c| {
            if let Some(client) = c.client() {
                client.request(Request::ClearLearned, |_| {});
            }
            c.refresh_state();
        })
    });
    st.on_set_start_with_windows(|v| with_ctx(|c| c.edit(|cfg| cfg.start_with_windows = v)));
    st.on_set_auto_connect(|v| with_ctx(|c| c.edit(|cfg| cfg.auto_connect = v)));
    st.on_set_tray_at_login(|v| {
        with_ctx(|c| {
            c.prefs.borrow_mut().tray_at_login = v;
            if !winsys::set_autorun(v) {
                c.toast("Oturum açılışı ayarı yazılamadı.", true);
            }
            c.save_prefs();
            c.apply_prefs();
        })
    });
    st.on_set_close_to_tray(|v| {
        with_ctx(|c| {
            c.prefs.borrow_mut().close_to_tray = v;
            c.save_prefs();
            c.apply_prefs();
        })
    });
    st.on_select_theme(|i| {
        with_ctx(|c| {
            c.prefs.borrow_mut().theme = i.clamp(0, 2) as u8;
            c.save_prefs();
            c.apply_prefs();
        })
    });
    st.on_open_url(|url| {
        if url.starts_with("https://") {
            winsys::shell_open(&url);
        }
    });
    st.on_open_logs(|| {
        let log = shared::paths::log_dir().join("service.log");
        if log.exists() {
            winsys::shell_open(&log.display().to_string());
        } else {
            with_ctx(|c| c.toast("Henüz günlük dosyası yok.", false));
        }
    });
}

/// Win11'de cercevesiz pencereye yuvarlak kose, DWM golgesi ve simge verir.
fn style_window(app: &AppWindow) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    app.window().with_winit_window(|w| {
        let Ok(handle) = w.window_handle() else { return };
        let RawWindowHandle::Win32(h) = handle.as_raw() else { return };
        let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
        unsafe {
            let corner = DWMWCP_ROUND;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                &corner as *const _ as *const core::ffi::c_void,
                size_of_val(&corner) as u32,
            );
            let margins = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
            DwmExtendFrameIntoClientArea(hwnd, &margins);
        }
        winsys::set_window_icons(hwnd);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_dns_inputs() {
        let a = parse_custom_dns("Ev", "9.9.9.9", "").unwrap();
        assert_eq!((a.kind, a.v4.as_str(), a.v4_port), (CustomDnsKind::Udp, "9.9.9.9", 53));
        let b = parse_custom_dns("", "77.88.8.8:1253", "").unwrap();
        assert_eq!((b.v4_port, b.name.as_str()), (1253, "Özel DNS"));
        let c = parse_custom_dns("v6", "[2a02:6b8::feed:ff]:1253", "").unwrap();
        assert_eq!(c.v6_port, 1253);
        let d = parse_custom_dns("NextDNS", "https://dns.nextdns.io/abc123", "45.90.28.0").unwrap();
        assert_eq!(d.kind, CustomDnsKind::Doh);
        assert!(parse_custom_dns("x", "https://dns.nextdns.io/abc", "").unwrap_err().contains("IP"));
        assert!(parse_custom_dns("x", "dns.google", "").is_err());
    }

    #[test]
    fn uptime_texts() {
        assert_eq!(uptime_text(Some(now_ms())), "Az önce bağlandı");
        assert_eq!(uptime_text(Some(now_ms() - 5 * 60_000)), "5 dakikadır korumalı");
        assert_eq!(uptime_text(Some(now_ms() - 125 * 60_000)), "2 sa 5 dk'dır korumalı");
    }
}
