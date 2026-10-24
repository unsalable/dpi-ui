//! Mac motor hizmetinin durumu. Istekler Windows servisindekiyle ayni (`shared::ipc`); farki
//! motor: WinDivert yerine yerel proxy + PAC dosyasi + sistem proxy ayari.
//!
//! Baglan: proxy baslar, PAC listedeki siteleri ona yonlendirir, sistem proxy'si PAC'i gosterir.
//! Kes: sistem ayari eski haline doner, proxy durur. Ayar degisince (yontem, liste, DNS) acik
//! baglanti yerinde guncellenir; PAC adresi her degisiklikte yeni surum numarasi alir ki
//! tarayicilar eski dosyayi onbellekten okumasin.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use proxy::pac::PacScope;
use proxy::{DohResolver, Options, Proxy, Resolve, SystemResolver, UdpResolver, presets};
use shared::config::{ActiveDns, AppConfig, UpdateChannel, UpdateMode};
use shared::ipc::{EngineState, EngineStatus, Event, Request, Response, ServerMessage, StateSnapshot, StatsSnapshot};
use shared::t;
use shared::update::{UpdateState, UpdateStatus, Version};

use crate::sysproxy::SystemProxy;
use crate::{error, info, search, updates, warn};

/// Kendiliginden guncelleme denetimi en sik bu kadar arayla.
const UPDATE_INTERVAL_MS: u64 = 6 * 3600 * 1000;

pub struct Controller {
    inner: Mutex<Inner>,
    me: Weak<Controller>,
    data_dir: PathBuf,
    /// Tercih edilen proxy portu (0: rastgele; testler).
    port: u16,
    /// "En iyi yontemi bul" istekleri (testlerde sahte).
    fetch: Arc<search::Fetch>,
}

struct Inner {
    config: AppConfig,
    status: EngineStatus,
    proxy: Option<Proxy>,
    sys: Box<dyn SystemProxy>,
    pac_version: u64,
    subscribers: Vec<(u64, SyncSender<ServerMessage>)>,
    update: UpdateStatus,
    update_url: Option<String>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn broadcast(inner: &mut Inner, event: Event) {
    let msg = ServerMessage::Event { event };
    // Dolu kuyruk (yavas istemci) mesaji kaybeder ama hizmeti bekletmez; kopan cikarilir.
    inner.subscribers.retain(|(_, tx)| !matches!(tx.try_send(msg.clone()), Err(std::sync::mpsc::TrySendError::Disconnected(_))));
}

/// Secili Mac yontemi (bos/bilinmeyen kimlikte varsayilan).
pub fn active_method(cfg: &AppConfig) -> &'static presets::Method {
    presets::method_or_default(&cfg.proxy_method)
}

fn resolver(cfg: &AppConfig) -> Result<Arc<dyn Resolve>, String> {
    let fallback = cfg.dns.fallback_to_system;
    Ok(match cfg.active_dns() {
        ActiveDns::Off => Arc::new(SystemResolver),
        ActiveDns::Encrypted { url, bootstrap, .. } => Arc::new(DohResolver::new(&url, &bootstrap, fallback)?),
        ActiveDns::Redirect { v4, v6, .. } => {
            let servers: Vec<SocketAddr> =
                v4.map(|(a, p)| SocketAddr::from((a, p))).into_iter().chain(v6.map(|(a, p)| SocketAddr::from((a, p)))).collect();
            Arc::new(UdpResolver::new(servers, fallback)?)
        }
    })
}

/// PAC'in proxy'ye yonlendirecegi siteler (Windows'taki kapsamla ayni kural).
pub fn pac_scope(cfg: &AppConfig) -> PacScope {
    let cats: Vec<&str> = cfg.scope.categories.iter().map(String::as_str).collect();
    let mut targets: Vec<String> = Vec::new();
    if !cats.is_empty() {
        targets.extend(engine::scope::builtin_targets(&cats).map(str::to_owned));
    }
    targets.extend(cfg.scope.targets.iter().cloned());
    PacScope { all: cfg.scope.mode == engine::ScopeMode::All, targets, exclude: cfg.scope.excluded.clone() }
}

impl Controller {
    pub fn new(config: AppConfig, sys: Box<dyn SystemProxy>, data_dir: PathBuf, port: u16) -> Arc<Self> {
        Self::with_fetch(config, sys, data_dir, port, Arc::new(search::curl))
    }

    /// `fetch`: yontem aramasinin istekleri (testlerde gercek siteler yerine sahte).
    pub fn with_fetch(config: AppConfig, sys: Box<dyn SystemProxy>, data_dir: PathBuf, port: u16, fetch: Arc<search::Fetch>) -> Arc<Self> {
        Arc::new_cyclic(|me| Controller {
            inner: Mutex::new(Inner {
                status: EngineStatus { method_name: active_method(&config).label(shared::i18n::english()).into(), dns_name: config.active_dns().display_name(), ..Default::default() },
                config,
                proxy: None,
                sys,
                pac_version: 0,
                subscribers: Vec::new(),
                update: UpdateStatus::default(),
                update_url: None,
            }),
            me: me.clone(),
            data_dir,
            port,
            fetch,
        })
    }

    pub fn config(&self) -> AppConfig {
        self.inner.lock().unwrap().config.clone()
    }

    pub fn status(&self) -> EngineStatus {
        self.inner.lock().unwrap().status.clone()
    }

    /// Calisan proxy'nin adresi (testler ve tanilama).
    pub fn proxy_addr(&self) -> Option<SocketAddr> {
        self.inner.lock().unwrap().proxy.as_ref().map(Proxy::local_addr)
    }

    pub fn unsubscribe(&self, id: u64) {
        self.inner.lock().unwrap().subscribers.retain(|(i, _)| *i != id);
    }

    pub fn handle(&self, req: Request, client: Option<(u64, &SyncSender<ServerMessage>)>) -> Response {
        match req {
            Request::Hello { client, protocol } => {
                info!("İstemci bağlandı: {client} (protokol {protocol}).");
                Response::Hello { service_version: shared::VERSION.into(), protocol: shared::ipc::PROTOCOL_VERSION }
            }
            Request::GetState => Response::State { state: Box::new(self.snapshot()) },
            Request::Subscribe => {
                if let Some((id, tx)) = client {
                    let mut inner = self.inner.lock().unwrap();
                    if inner.subscribers.iter().all(|(i, _)| *i != id) {
                        inner.subscribers.push((id, tx.clone()));
                    }
                }
                Response::Ok
            }
            Request::Connect => match self.connect() {
                Ok(()) => Response::Ok,
                Err(message) => Response::Error { message },
            },
            Request::Disconnect => {
                self.disconnect();
                Response::Ok
            }
            Request::SetConfig { config } => match self.set_config(*config) {
                Ok(()) => Response::Ok,
                Err(message) => Response::Error { message },
            },
            Request::GetStats => Response::Stats { stats: self.stats() },
            // Mac motoru (PAC ile yalnizca listedeki siteler) ogrenme yapmaz.
            Request::ForgetLearned { .. } | Request::ClearLearned => Response::Ok,
            Request::GetLogs => Response::Logs { lines: crate::log::lines() },
            Request::BenchDns => Response::DnsBench { results: crate::bench::run_all() },
            Request::FindMethod => self.find_method(),
            Request::CheckUpdate { forced } => {
                self.check_update(forced);
                Response::Ok
            }
            Request::InstallUpdate => match self.inner.lock().unwrap().update_url.clone() {
                Some(url) => {
                    // Kurulum paketi tarayicida iner; kullanici uygulamayi Uygulamalar'a surukler.
                    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
                    Response::Ok
                }
                None => Response::Error { message: t!("Kurulacak güncelleme yok.", "There's no update to install.") },
            },
        }
    }

    pub fn snapshot(&self) -> StateSnapshot {
        let inner = self.inner.lock().unwrap();
        StateSnapshot {
            service_version: shared::VERSION.into(),
            engine: inner.status.clone(),
            config: inner.config.clone(),
            learned: Vec::new(),
            conflicts: Vec::new(),
            update: inner.update.clone(),
        }
    }

    fn stats(&self) -> StatsSnapshot {
        let inner = self.inner.lock().unwrap();
        let Some(p) = inner.proxy.as_ref() else { return StatsSnapshot::default() };
        let s = p.stats();
        StatsSnapshot { bypassed: s.desynced, untouched: s.accepted.saturating_sub(s.desynced), engine_errors: s.failed, ..Default::default() }
    }

    fn pac_url(inner: &Inner, addr: SocketAddr) -> String {
        format!("http://{addr}{}?v={}", proxy::PAC_PATH, inner.pac_version)
    }

    pub fn connect(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        match self.start(&mut inner) {
            Ok(()) => Ok(()),
            Err(message) => {
                error!("Bağlanılamadı: {message}");
                Self::stop(&mut inner);
                inner.status = EngineStatus { state: EngineState::Error, message: Some(message.clone()), ..Self::names(&inner.config) };
                let status = inner.status.clone();
                broadcast(&mut inner, Event::Engine { status });
                Err(message)
            }
        }
    }

    fn names(cfg: &AppConfig) -> EngineStatus {
        EngineStatus {
            method_name: active_method(cfg).label(shared::i18n::english()).into(),
            dns_name: cfg.active_dns().display_name(),
            ..Default::default()
        }
    }

    fn start(&self, inner: &mut Inner) -> Result<(), String> {
        if let Some(p) = inner.proxy.take() {
            p.stop();
        }
        let resolver = resolver(&inner.config)?;
        let desync = active_method(&inner.config).desync();
        let mut opts = Options::new(self.port, desync.clone(), resolver.clone());
        let proxy = match Proxy::start(opts) {
            Ok(p) => p,
            Err(e) if self.port != 0 => {
                // Port baska bir uygulamada: rastgele porta gec (PAC adresi onu gosterir).
                warn!("{} portu kullanılamadı ({e}); rastgele port deneniyor.", self.port);
                opts = Options::new(0, desync, resolver);
                Proxy::start(opts).map_err(|e| t!("Proxy başlatılamadı: {e}", "Couldn't start the proxy: {e}"))?
            }
            Err(e) => return Err(t!("Proxy başlatılamadı: {e}", "Couldn't start the proxy: {e}")),
        };
        let addr = proxy.local_addr();
        proxy.set_pac(Some(proxy::pac::script(addr, &pac_scope(&inner.config))));
        inner.pac_version += 1;
        let url = Self::pac_url(inner, addr);
        inner.proxy = Some(proxy);
        inner.sys.enable(&url).map_err(|e| t!("Sistem proxy ayarı yapılamadı: {e}", "Couldn't set the system proxy: {e}"))?;
        info!("Bağlandı: {addr}, yöntem {}, PAC {url}", active_method(&inner.config).id);
        inner.status = EngineStatus { state: EngineState::On, since_ms: Some(now_ms()), ..Self::names(&inner.config) };
        let status = inner.status.clone();
        broadcast(inner, Event::Engine { status });
        Ok(())
    }

    fn stop(inner: &mut Inner) {
        if let Err(e) = inner.sys.disable() {
            error!("Sistem proxy ayarı geri alınamadı: {e}");
        }
        if let Some(p) = inner.proxy.take() {
            p.stop();
        }
    }

    pub fn disconnect(&self) {
        let mut inner = self.inner.lock().unwrap();
        let was = inner.status.state;
        Self::stop(&mut inner);
        inner.status = EngineStatus { state: EngineState::Off, ..Self::names(&inner.config) };
        if was != EngineState::Off {
            info!("Bağlantı kesildi.");
        }
        let status = inner.status.clone();
        broadcast(&mut inner, Event::Engine { status });
    }

    pub fn set_config(&self, new: AppConfig) -> Result<(), String> {
        let mut new = new.sanitized();
        new.validate()?;
        if !new.proxy_method.is_empty() && presets::method(&new.proxy_method).is_none() {
            return Err(t!("Seçili yöntem bulunamadı.", "The selected method wasn't found."));
        }
        new.proxy_method = active_method(&new).id.to_owned();
        crate::store::save_config(&self.data_dir, &new).map_err(|e| t!("Ayar kaydedilemedi: {e}", "Couldn't save the settings: {e}"))?;
        shared::i18n::set(new.language);

        let mut inner = self.inner.lock().unwrap();
        let old = std::mem::replace(&mut inner.config, new.clone());
        broadcast(&mut inner, Event::Config { config: Box::new(new.clone()) });

        let running = inner.proxy.is_some() && inner.status.state == EngineState::On;
        let dns_changed = old.active_dns() != new.active_dns() || old.dns.fallback_to_system != new.dns.fallback_to_system;
        if running && dns_changed {
            // Cozucu proxy'yle birlikte kurulur: yeniden baslat.
            drop(inner);
            return self.connect();
        }
        if running {
            let proxy = inner.proxy.as_ref().unwrap();
            if active_method(&old).id != active_method(&new).id {
                proxy.set_desync(active_method(&new).desync());
                info!("Yöntem değişti: {}", active_method(&new).id);
            }
            if pac_scope(&old) != pac_scope(&new) {
                let addr = proxy.local_addr();
                proxy.set_pac(Some(proxy::pac::script(addr, &pac_scope(&new))));
                inner.pac_version += 1;
                let url = Self::pac_url(&inner, addr);
                if let Err(e) = inner.sys.enable(&url) {
                    warn!("Yeni PAC adresi sisteme yazılamadı: {e}");
                }
            }
        }
        let state = inner.status.state;
        inner.status = EngineStatus { state, since_ms: inner.status.since_ms, message: inner.status.message.clone(), ..Self::names(&new) };
        let status = inner.status.clone();
        broadcast(&mut inner, Event::Engine { status });
        Ok(())
    }

    /// Yontemleri bu hatta dener; engel varsa en iyisini secip kaydeder.
    pub fn find_method(&self) -> Response {
        let cfg = self.config();
        let resolver = match resolver(&cfg) {
            Ok(r) => r,
            Err(message) => return Response::Error { message },
        };
        let out = search::run(resolver, self.fetch.clone(), shared::i18n::english());
        let summary: Vec<String> = out.trials.iter().map(|t| format!("{} {}/{}", t.id, t.opened, t.total)).collect();
        info!("Yöntem araması: yöntemsiz {}/{}, {}", out.baseline, search::TARGETS.len(), summary.join(", "));
        if !out.online && out.baseline == 0 && out.best().is_none() {
            return Response::Error { message: t!("İnternete ulaşılamadı; bağlantını kontrol et.", "Couldn't reach the internet; check your connection.") };
        }
        let blocked = out.blocked();
        let best = if blocked { out.best().map(|t| t.id.clone()) } else { None };
        if let Some(id) = &best {
            let mut c = cfg;
            c.proxy_method = id.clone();
            if let Err(message) = self.set_config(c) {
                return Response::Error { message };
            }
        }
        Response::MethodSearch { best, blocked, results: out.trials }
    }

    fn set_update(&self, state: UpdateState, url: Option<String>, finished: bool) {
        let mut inner = self.inner.lock().unwrap();
        if finished && !matches!(state, UpdateState::Failed { .. }) {
            inner.update.checked_at_ms = Some(now_ms());
        }
        inner.update.state = state;
        inner.update_url = url;
        let status = inner.update.clone();
        broadcast(&mut inner, Event::Update { status });
    }

    /// Guncellemeyi arka planda denetler; sonuc `Event::Update` olarak gider.
    pub fn check_update(&self, forced: bool) {
        let (mode, channel, last) = {
            let inner = self.inner.lock().unwrap();
            (inner.config.updates, inner.config.update_channel, inner.update.checked_at_ms)
        };
        if !forced && (mode == UpdateMode::Off || last.is_some_and(|t| now_ms().saturating_sub(t) < UPDATE_INTERVAL_MS)) {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        self.set_update(UpdateState::Checking, None, false);
        std::thread::spawn(move || {
            let current = Version::parse(shared::VERSION).expect("surum");
            match updates::fetch() {
                Ok(list) => match updates::pick(&current, &list, channel == UpdateChannel::Beta) {
                    Some(f) => {
                        info!("Mac güncellemesi var: {}", f.version);
                        me.set_update(UpdateState::Ready { version: f.version.to_string(), notes: f.notes, sha256: String::new() }, Some(f.url), true)
                    }
                    None => me.set_update(UpdateState::UpToDate, None, true),
                },
                Err(message) => {
                    warn!("Güncelleme denetlenemedi: {message}");
                    me.set_update(UpdateState::Failed { message }, None, true)
                }
            }
        });
    }
}
