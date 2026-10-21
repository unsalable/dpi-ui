//! Pencere sureci: Slint arayuzu + servis durumunun ekrana yansitilmasi.
//! Tum ayar degisiklikleri servise gonderilir; ekran servisten gelen olaylarla guncellenir
//! (tek dogruluk kaynagi servis).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use engine::presets;
use shared::config::{AppConfig, CustomDns, CustomDnsKind, DnsMode, UpdateMode};
use shared::i18n::{self, Language};
use shared::t;
use shared::update::{UpdateState, UpdateStatus};
use shared::ipc::{EngineState, EngineStatus, Event, LearnedHost, Request, Response, StateSnapshot};
use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::client::{Client, Incoming};
use crate::prefs::{self, Prefs};
use crate::winsys;
use crate::{AppState, AppWindow, BenchItem, CategoryItem, Choice, HostItem, Motion, PowerState, Theme};

/// Dokununca tarama animasyonu en az bu kadar gorunur (sonuc daha once gelse bile).
const MIN_BUSY: Duration = Duration::from_millis(750);
/// Baglanma istegine bu sure icinde hic yanit gelmezse gercek durum gosterilir.
const MAX_PENDING: Duration = Duration::from_secs(6);
/// Guncelleme kurulurken pencere bu kadar sonra kapanir (bildirim okunabilsin; servis 5 sn bekler).
const UPDATE_CLOSE_DELAY: Duration = Duration::from_millis(1500);

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
    /// Kullanici "baglan" dedi; sonuc gelene (ve en az MIN_BUSY gecene) kadar tarama gosterilir.
    pending_connect: Cell<Option<Instant>>,
    /// Kapanma basladi.
    closing: Cell<bool>,
    update: RefCell<UpdateStatus>,
    /// Servis guncellemeyi kuruyor; pencere kapanmak uzere.
    update_closing: Cell<bool>,
    /// Kullanici "simdi denetle" dedi: hata olursa bildirim gosterilir.
    update_requested: Cell<bool>,
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
    fn new(app: &AppWindow, prefs: Prefs, test_mode: bool) -> Self {
        Ctx {
            app: app.as_weak(),
            client: RefCell::new(None),
            config: RefCell::new(None),
            engine: RefCell::new(EngineStatus::default()),
            conflicts: RefCell::new(Vec::new()),
            learned: RefCell::new(Vec::new()),
            connected: Cell::new(false),
            prefs: RefCell::new(prefs),
            toast_serial: Cell::new(0),
            method_ids: RefCell::new(Vec::new()),
            dns_ids: RefCell::new(Vec::new()),
            pending_connect: Cell::new(None),
            closing: Cell::new(false),
            update: RefCell::new(UpdateStatus::default()),
            update_closing: Cell::new(false),
            update_requested: Cell::new(false),
            test_mode,
        }
    }

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
            self.toast(&t!("Motor hizmetine bağlı değil.", "Not connected to the engine service."), true);
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
                Event::Config { config } => {
                    self.apply_language(config.language);
                    self.apply_config(*config);
                }
                Event::Learned { host } => {
                    self.toast(&t!("{} açılmıyordu; artık otomatik atlatılıyor.", "{} wasn't loading; it's now bypassed automatically.", host.host), false);
                    self.learned.borrow_mut().push(host);
                    self.apply_learned();
                }
                Event::Conflicts { conflicts } => {
                    *self.conflicts.borrow_mut() = conflicts;
                    self.apply_engine();
                }
                Event::Update { status } => {
                    let failed_now = matches!(status.state, UpdateState::Failed { .. })
                        && !matches!(self.update.borrow().state, UpdateState::Failed { .. });
                    *self.update.borrow_mut() = status;
                    self.apply_update();
                    if failed_now && self.update_requested.get() {
                        if let UpdateState::Failed { message } = &self.update.borrow().state {
                            self.toast(&t!("Güncelleme denetlenemedi: {message}", "Couldn't check for updates: {message}"), true);
                        }
                    }
                    self.close_for_update();
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
        *self.update.borrow_mut() = state.update;
        self.apply_language(state.config.language);
        self.apply_config(state.config);
        self.apply_learned();
        self.apply_engine();
        self.apply_update();
        self.first_run_tasks(fresh);
        // Uygulama acilinca ilk is guncellemeyi denetlemek (servis yakin zamanda baktiysa aga cikmaz).
        if let Some(client) = self.client() {
            client.send(Request::CheckUpdate { forced: false });
        }
        self.close_for_update();
    }

    fn apply_update(&self) {
        let Some(app) = self.ui() else { return };
        let st = app.global::<AppState>();
        let u = self.update.borrow();
        let (text, ready, busy) = match &u.state {
            UpdateState::Idle => (t!("Henüz denetlenmedi", "Not checked yet"), false, false),
            UpdateState::Checking => (t!("Denetleniyor…", "Checking…"), false, true),
            UpdateState::UpToDate => (t!("Güncel", "Up to date"), false, false),
            UpdateState::Downloading { version, percent } => (t!("{version} indiriliyor · %{percent}", "Downloading {version} · {percent}%"), false, true),
            UpdateState::Ready { version, .. } => (t!("{version} hazır", "{version} ready"), true, false),
            UpdateState::Installing { version } => (t!("{version} yükleniyor…", "Installing {version}…"), false, true),
            UpdateState::Failed { .. } => (t!("Denetlenemedi", "Couldn't check"), false, false),
        };
        st.set_update_text(s(text));
        st.set_update_ready(ready);
        st.set_update_busy(busy);
    }

    /// "Simdi yukle" ("Otomatik yukle" kapaliyken). Kurulumu servis sessizce yapar.
    fn install_update(&self) {
        let Some(client) = self.client() else {
            self.toast(&t!("Motor hizmetine bağlı değil.", "Not connected to the engine service."), true);
            return;
        };
        client.request(Request::InstallUpdate, |r| {
            if let Response::Error { message } = r {
                post_toast(message, true);
            }
        });
    }

    /// Servis guncellemeyi kuruyor: pencerenin dosyasi degisecegi icin pencere kapanir;
    /// kurulumdan sonra servis pencereyi yeniden acar.
    fn close_for_update(&self) {
        let UpdateState::Installing { version } = &self.update.borrow().state else { return };
        if self.closing.get() || self.update_closing.replace(true) {
            return;
        }
        self.toast(
            &t!(
                "Sürüm {version} yükleniyor; uygulama birkaç saniye içinde yeniden açılacak.",
                "Installing version {version}; the app will reopen in a few seconds."
            ),
            false,
        );
        Timer::single_shot(UPDATE_CLOSE_DELAY, || with_ctx(|c| c.quit_now()));
    }

    fn apply_engine(&self) {
        let Some(app) = self.ui() else { return };
        let st = app.global::<AppState>();
        let e = self.engine.borrow();
        st.set_service_missing(!self.connected.get());
        if !self.connected.get() {
            st.set_power(PowerState::Off);
            st.set_status_title(s(t!("Motor hizmeti kapalı", "Engine service is off")));
            st.set_status_sub(s(t!("Başlatmak için dokun", "Tap to start")));
            st.set_banner(s(""));
            return;
        }
        let (mut power, mut title, mut sub) = match e.state {
            EngineState::Off => (PowerState::Off, t!("Kapalı", "Off"), t!("Bağlanmak için dokun", "Tap to connect")),
            EngineState::Starting => (PowerState::Connecting, t!("Bağlanıyor…", "Connecting…"), String::new()),
            EngineState::On => (PowerState::On, t!("Korumalı", "Protected"), String::new()),
            EngineState::Paused => (
                PowerState::Off,
                t!("Duraklatıldı", "Paused"),
                e.message.clone().unwrap_or_else(|| t!("Bir uygulama için geçici olarak durduruldu", "Temporarily stopped for an app")),
            ),
            EngineState::Error => (
                PowerState::Error,
                t!("Bağlanamadı", "Couldn't connect"),
                e.message.clone().unwrap_or_else(|| t!("Bilinmeyen hata", "Unknown error")),
            ),
        };
        if let Some(since) = self.pending_connect.get() {
            let elapsed = since.elapsed();
            let settled = matches!(e.state, EngineState::On | EngineState::Error | EngineState::Paused);
            let hold = if settled { elapsed < MIN_BUSY } else { elapsed < MAX_PENDING };
            if hold {
                if settled {
                    Timer::single_shot(MIN_BUSY - elapsed, || with_ctx(|c| c.apply_engine()));
                }
                (power, title, sub) = (PowerState::Connecting, t!("Bağlanıyor…", "Connecting…"), String::new());
            } else {
                self.pending_connect.set(None);
            }
        }
        st.set_power(power);
        st.set_status_title(s(title));
        st.set_status_sub(s(sub));
        st.set_method_name(s(e.method_name.as_str()));
        st.set_dns_name(s(e.dns_name.as_str()));

        let conflicts = self.conflicts.borrow();
        let banner = if !conflicts.is_empty() {
            let names: Vec<&str> = conflicts.iter().map(|c| c.description.as_str()).collect();
            t!(
                "{}. Aynı anda iki DPI aracı bağlantıları bozabilir; diğerini kapat.",
                "{}. Running two DPI tools at once can break connections; close the other one.",
                names.join(", ")
            )
        } else if e.state == EngineState::On {
            e.message.clone().unwrap_or_default()
        } else {
            String::new()
        };
        st.set_banner(s(banner));
        if e.state != EngineState::On {
        }
    }

    fn apply_config(&self, cfg: AppConfig) {
        let Some(app) = self.ui() else { return };
        let st = app.global::<AppState>();

        let en = i18n::english();
        // Saglayicilar.
        let isps: Vec<Choice> = presets::ISPS
            .iter()
            .map(|i| Choice { id: s(i.id), title: s(i.label(en)), subtitle: s(i.about(en)), badge: s("") })
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
                let badge = if n == 0 { t!("Önerilen", "Recommended") } else { String::new() };
                items.push(Choice { id: s(m.id), title: s(m.label(en)), subtitle: s(m.config().short_summary(en)), badge: s(badge) });
                ids.push(m.id.into());
            }
        }
        for m in presets::ALL.iter().filter(|m| !isp.methods.contains(&m.id)) {
            items.push(Choice { id: s(m.id), title: s(m.label(en)), subtitle: s(m.config().short_summary(en)), badge: s("") });
            ids.push(m.id.into());
        }
        for c in &cfg.custom_methods {
            items.push(Choice {
                id: s(c.id.as_str()),
                title: s(t!("Özel · {}", "Custom · {}", c.name)),
                subtitle: s(c.config.short_summary(en)),
                badge: s(""),
            });
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
                        format!("{} · {a}:{port}", p.about(en))
                    }
                    _ => format!("{} · {}", p.about(en), if p.doh_url.starts_with("tls://") { "DoT" } else { "DoH" }),
                };
                providers.push(Choice { id: s(p.id), title: s(p.label(en)), subtitle: s(detail), badge: s("") });
                dns_ids.push(p.id.to_owned());
            }
        }
        for c in &cfg.dns.custom {
            let fits = matches!((cfg.dns.mode, c.kind), (DnsMode::Encrypted, CustomDnsKind::Doh) | (DnsMode::Redirect, CustomDnsKind::Udp));
            if fits {
                providers.push(Choice {
                    id: s(c.id.as_str()),
                    title: s(t!("Özel · {}", "Custom · {}", c.name)),
                    subtitle: s(custom_dns_address(c)),
                    badge: s(""),
                });
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
        st.set_auto_update(cfg.updates == UpdateMode::Auto);
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
            .map(|h| HostItem { key: s(h.host.as_str()), name: s(h.host.as_str()), note: s(learn_reason(&h.reason)) })
            .collect();
        app.global::<AppState>().set_learned(model(items));
    }

    /// Dili uygular: Slint ekranlari (`@tr`) ve Rust'in urettigi metinler birlikte degisir.
    /// Son secim tercihlere de yazilir; pencere bir sonraki acilista ayar gelmeden o dilde acilir.
    fn apply_language(&self, language: Language) {
        let was_english = i18n::english();
        i18n::set(language);
        let _ = slint::select_bundled_translation(if i18n::english() { "en" } else { "" });
        if let Some(app) = self.ui() {
            let index = match language {
                Language::System => 0,
                Language::Turkish => 1,
                Language::English => 2,
            };
            app.global::<AppState>().set_language_index(index);
        }
        if self.prefs.borrow().language != language && !self.test_mode {
            self.prefs.borrow_mut().language = language;
            self.save_prefs();
        }
        if was_english != i18n::english() {
            let cfg = self.config.borrow().clone();
            if let Some(cfg) = cfg {
                self.apply_config(cfg);
            }
            self.apply_engine();
            self.apply_update();
            self.apply_learned();
        }
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
        if let Some(hwnd) = hwnd_of(&app) {
            winsys::set_window_theme(hwnd, dark);
        }
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
                            let summary = if m.summary.is_empty() { t!("varsayılanlar", "defaults") } else { m.summary.join(", ") };
                            client.request(Request::SetConfig { config: Box::new(m.config) }, move |r| match r {
                                Response::Ok => post_toast(t!("Eski sürüm ayarların aktarıldı: {summary}.", "Your settings from the old version were imported: {summary}."), false),
                                Response::Error { message } => post_toast(t!("Eski ayarlar aktarılamadı: {message}", "Couldn't import the old settings: {message}"), true),
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
        // Tarama surerken ikinci dokunus vazgecmektir.
        let was_pending = self.pending_connect.take().is_some();
        let connect = !was_pending && !matches!(state, EngineState::On | EngineState::Starting);
        if connect {
            // Simge hemen taramaya gecer; sonuc gelince (en erken MIN_BUSY sonra) yerine oturur.
            self.pending_connect.set(Some(Instant::now()));
            Timer::single_shot(MAX_PENDING, || with_ctx(|c| c.apply_engine()));
        }
        self.apply_engine();
        client.request(if connect { Request::Connect } else { Request::Disconnect }, |r| {
            if let Response::Error { message } = r {
                let _ = slint::invoke_from_event_loop(move || {
                    with_ctx(|c| {
                        c.pending_connect.set(None);
                        c.apply_engine();
                        c.toast(&message, true);
                    })
                });
            }
        });
    }

    /// Kapat dugmesi ve Alt+F4: tercihe gore bildirim alaninda kalir ya da her seyi kapatir.
    fn close_window(&self) {
        if self.closing.get() {
            return;
        }
        if self.prefs.borrow().close_to_tray {
            if !self.test_mode {
                spawn_tray();
            }
        } else {
            winsys::signal_quit();
        }
        self.quit_now();
    }

    /// Pencereyi gizler ve sureci bitirir. (Solma bilerek yok: pencereyi katmanli moda almak
    /// Windows'ta icerigi siler, uygulama yalnizca degisen yeri yeniden cizdigi icin pencere bir
    /// an bos gorunuyordu.)
    fn quit_now(&self) {
        if self.closing.replace(true) {
            return;
        }
        if let Some(app) = self.ui() {
            let _ = app.hide();
        }
        let _ = slint::quit_event_loop();
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
                                        note: s(if ok { format!("{ms} ms") } else { t!("hata", "error") }),
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
            return t!("Geçerli bir alan adı gir (örn. site.com).", "Enter a valid domain (e.g. site.com).");
        };
        let exists = self.config.borrow().as_ref().is_some_and(|c| {
            let list = if excluded { &c.scope.excluded } else { &c.scope.targets };
            list.iter().any(|h| h == &name)
        });
        if exists {
            return t!("Zaten listede.", "Already in the list.");
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

}

fn category_title(id: &str) -> String {
    match id {
        "discord" => "Discord".into(),
        "roblox" => "Roblox".into(),
        "wattpad" => "Wattpad".into(),
        "vpn" => t!("VPN sağlayıcılarının siteleri", "VPN providers' sites"),
        _ => t!("Diğer", "Other"),
    }
}

/// Servisin ogrenilen siteye yazdigi neden (kayitta Turkce durur).
fn learn_reason(reason: &str) -> String {
    match reason {
        "yanıtsız kaldı" => t!("yanıtsız kaldı", "timed out"),
        "bağlantı sıfırlandı" => t!("bağlantı sıfırlandı", "connection reset"),
        other => other.to_owned(),
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
        return Err(t!("Bir adres gir.", "Enter an address."));
    }
    let name = if name.trim().is_empty() { t!("Özel DNS", "Custom DNS") } else { name.trim().to_owned() };
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
        return Err(t!(
            "Adres anlaşılamadı. IP (1.2.3.4), IP:port ya da https://… / tls://… gir.",
            "Couldn't understand the address. Enter an IP (1.2.3.4), IP:port or https://… / tls://…."
        ));
    }
    d.validate()?;
    Ok(d)
}

// ================================================================ pencere

pub fn run(show_event: windows_sys::Win32::Foundation::HANDLE) -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;
    let ctx = Rc::new(Ctx::new(&app, prefs::load(), std::env::var_os("GDPI3_TEST").is_some()));
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    let language = ctx.prefs.borrow().language;
    ctx.apply_language(language);
    app.global::<Motion>().set_enabled(winsys::animations_enabled());
    ctx.apply_prefs();
    ctx.apply_engine();
    app.global::<AppState>().set_version(s(shared::VERSION));

    bind_callbacks(&app);

    // Windows simge durumuna kucultmede (ve bazen ortulup acilinca) pencere icerigini atar;
    // Slint ise yalnizca degisen yerleri yeniden cizer. Pencere geri gelince tamami cizilsin.
    app.window().on_winit_window_event({
        let weak = app.as_weak();
        move |_, event| {
            use winit::event::WindowEvent;
            let restore = match event {
                WindowEvent::Resized(size) => size.width > 0 && size.height > 0,
                WindowEvent::Occluded(false) | WindowEvent::Focused(true) => true,
                _ => false,
            };
            if restore {
                let weak = weak.clone();
                Timer::single_shot(Duration::from_millis(0), move || {
                    if let Some(app) = weak.upgrade() {
                        app.set_repaint_epoch(app.get_repaint_epoch().wrapping_add(1));
                    }
                });
            }
            slint::winit_030::EventResult::Propagate
        }
    });

    // Servis baglantisi: olaylar IPC is parcacigindan arayuz dongusune aktarilir.
    let client = Client::start("pencere", |inc| {
        let _ = slint::invoke_from_event_loop(move || with_ctx(|c| c.on_incoming(inc)));
    });
    *ctx.client.borrow_mut() = Some(client);

    // Ikinci kez acilinca var olan pencere one gelir; tepsiden "Cik" denince pencere de kapanir.
    let quit = winsys::quit_event() as usize;
    let show = show_event as usize;
    std::thread::Builder::new()
        .name("tek-ornek".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            use windows_sys::Win32::System::Threading::{INFINITE, WaitForMultipleObjects};
            let handles = [quit as windows_sys::Win32::Foundation::HANDLE, show as _];
            loop {
                let r = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
                if r == WAIT_OBJECT_0 {
                    let _ = slint::invoke_from_event_loop(|| with_ctx(|c| c.quit_now()));
                    return;
                }
                if r != WAIT_OBJECT_0 + 1 {
                    return;
                }
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
            }
        })
        .ok();

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
        }
        if std::env::var_os("GDPI3_LIGHT").is_some() {
            app.global::<Theme>().set_dark(false);
        }
        // Gercek acilistaki gibi tercihler (ve pencere cercevesi) sonradan yeniden uygulansin:
        // servisten durum gelince first_run_tasks bunu yapar; test modunda o adim atlanir.
        if let Some(ms) = std::env::var("GDPI3_REAPPLY_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
            Timer::single_shot(Duration::from_millis(ms), || with_ctx(|c| c.apply_prefs()));
        }
    }

    app.show()?;
    // Slint winit penceresini olay dongusu baslayinca olusturur; o ana kadar HWND yoktur.
    // Yuvarlak kose, golge, simge ve koyu cerceve pencere olusur olusmaz uygulanir.
    let styler = Rc::new(Timer::default());
    let tries = Cell::new(0);
    let weak = app.as_weak();
    let keep = Rc::downgrade(&styler);
    styler.start(TimerMode::Repeated, Duration::from_millis(10), move || {
        tries.set(tries.get() + 1);
        let done = weak.upgrade().is_none_or(|app| style_window(&app));
        if done || tries.get() > 200 {
            if let Some(t) = keep.upgrade() {
                t.stop();
            }
        }
    });
    slint::run_event_loop()?;
    drop(styler);
    CTX.with(|c| *c.borrow_mut() = None);
    Ok(())
}

fn spawn_tray() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--tray").spawn();
    }
}

fn bind_callbacks(app: &AppWindow) {
    app.global::<AppState>().on_drag_window({
        let weak = app.as_weak();
        move || {
            // Sol tus gercekten basili degilse suruklemeye girilmez: birakildiktan sonra baslayan
            // surukleme pencereyi bir sonraki tiklamaya kadar imlece yapistirir ve dongu durur.
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
            if unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } as u16 & 0x8000 == 0 {
                return;
            }
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
    app.on_close_window(|| with_ctx(|c| c.close_window()));
    app.window().on_close_requested(|| {
        with_ctx(|c| c.close_window());
        slint::CloseRequestResponse::KeepWindowShown
    });

    let st = app.global::<AppState>();
    st.on_power_toggle(|| with_ctx(|c| c.power_toggle()));
    st.on_start_service(|| {
        with_ctx(|c| match winsys::start_service() {
            Ok(()) => c.toast(&t!("Motor hizmeti başlatılıyor…", "Starting the engine service…"), false),
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
                let en = i18n::english();
                c.toast(&t!("{} için önerilen yöntem seçildi: {}.", "Recommended method for {} selected: {}.", isp.label(en), rec.label(en)), false);
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
            c.toast(&t!("DNS değiştirildi.", "DNS changed."), false);
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
    st.on_set_auto_update(|v| with_ctx(|c| c.edit(|cfg| cfg.updates = if v { UpdateMode::Auto } else { UpdateMode::Notify })));
    st.on_check_update(|| {
        with_ctx(|c| {
            let Some(client) = c.client() else {
                c.toast(&t!("Motor hizmetine bağlı değil.", "Not connected to the engine service."), true);
                return;
            };
            c.update_requested.set(true);
            client.send(Request::CheckUpdate { forced: true });
        })
    });
    st.on_install_update(|| with_ctx(|c| c.install_update()));
    st.on_set_auto_connect(|v| with_ctx(|c| c.edit(|cfg| cfg.auto_connect = v)));
    st.on_set_tray_at_login(|v| {
        with_ctx(|c| {
            c.prefs.borrow_mut().tray_at_login = v;
            if !winsys::set_autorun(v) {
                c.toast(&t!("Oturum açılışı ayarı yazılamadı.", "Couldn't save the sign-in setting."), true);
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
    st.on_select_language(|i| {
        let language = match i {
            1 => Language::Turkish,
            2 => Language::English,
            _ => Language::System,
        };
        with_ctx(|c| {
            c.apply_language(language);
            c.edit(|cfg| cfg.language = language);
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
            with_ctx(|c| c.toast(&t!("Henüz günlük dosyası yok.", "There's no log file yet."), false));
        }
    });
}

/// Win11'de cercevesiz pencereye yuvarlak kose, DWM golgesi ve simge verir. Pencere henuz
/// olusmadiysa false doner.
fn style_window(app: &AppWindow) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::Controls::MARGINS;

    let Some(hwnd) = hwnd_of(app) else { return false };
    {
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
        winsys::set_window_theme(hwnd, app.global::<Theme>().get_dark());
    }
    true
}

fn hwnd_of(app: &AppWindow) -> Option<windows_sys::Win32::Foundation::HWND> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    app.window()
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get() as windows_sys::Win32::Foundation::HWND),
            _ => None,
        })
        .flatten()
}

/// Arayuz testleri: ekrani gercek ayarlarla (hazir saglayici/yontem/DNS listeleri) doldurur.
#[cfg(test)]
pub(crate) fn populate_for_test(app: &AppWindow, engine: EngineStatus, learned: Vec<LearnedHost>) {
    let ctx = Ctx::new(app, Prefs::default(), true);
    ctx.connected.set(true);
    *ctx.engine.borrow_mut() = engine;
    *ctx.learned.borrow_mut() = learned;
    let mut cfg = AppConfig::default().sanitized();
    cfg.scope.targets = vec!["ornek-site.com".into()];
    cfg.scope.excluded = vec!["banka.com.tr".into()];
    ctx.apply_config(cfg);
    ctx.apply_learned();
    ctx.apply_engine();
    app.global::<AppState>().set_version(s(shared::VERSION));
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

    /// Dokununca tarama en az MIN_BUSY gorunur; hic yanit gelmezse MAX_PENDING sonra gercek durum.
    #[test]
    fn connecting_is_shown_long_enough() {
        let h = crate::uitest::Harness::new(1.0);
        let ctx = Ctx::new(&h.app, Prefs::default(), true);
        let power = || h.app.global::<AppState>().get_power();
        ctx.connected.set(true);

        ctx.engine.borrow_mut().state = EngineState::On;
        ctx.pending_connect.set(Some(Instant::now()));
        ctx.apply_engine();
        assert_eq!(power(), PowerState::Connecting, "sonuc hemen geldi ama tarama surmeli");
        ctx.pending_connect.set(Some(Instant::now() - MIN_BUSY));
        ctx.apply_engine();
        assert_eq!(power(), PowerState::On);
        assert!(ctx.pending_connect.get().is_none());

        ctx.engine.borrow_mut().state = EngineState::Off;
        ctx.pending_connect.set(Some(Instant::now() - Duration::from_secs(2)));
        ctx.apply_engine();
        assert_eq!(power(), PowerState::Connecting, "motor henuz yanit vermedi");
        ctx.pending_connect.set(Some(Instant::now() - MAX_PENDING));
        ctx.apply_engine();
        assert_eq!(power(), PowerState::Off, "yanit gelmezse gercek durum gosterilmeli");
    }
}
