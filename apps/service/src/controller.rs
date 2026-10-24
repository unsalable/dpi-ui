//! Servisin merkezi durumu: ayar, motor, ogrenilenler ve abone istemciler.
//! Tum degisiklikler buradan gecer; arayuzler yalnizca istek gonderir.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use engine::{DnsMode, Processor, Redirect};
use shared::config::{ActiveDns, AppConfig};
use shared::ipc::{Conflict, EngineState, EngineStatus, Event, LearnedHost, Request, Response, ServerMessage, StateSnapshot, StatsSnapshot};
use shared::t;
use shared::update::{UpdateState, UpdateStatus};

use crate::engine_loop::{self, EngineMsg, RunningEngine};
use crate::updater::{self, UiSession, Updater};
use crate::{conflicts, error, info, scm, store, warn};

/// Cakisan bir arac calistigi surece listenin yenilenme araligi (arac kapaninca uyari kalksin).
const CONFLICT_POLL: Duration = Duration::from_secs(2);

pub struct Controller {
    inner: Mutex<Inner>,
    tx: Sender<EngineMsg>,
    exe_dir: PathBuf,
    as_service: bool,
    me: Weak<Controller>,
    updater: Updater,
    update: Mutex<UpdateStatus>,
    /// Cakisma izleyicisi calisiyor mu (yalnizca liste doluyken calisir).
    conflict_watch: AtomicBool,
}

struct Inner {
    config: AppConfig,
    learned: Vec<LearnedHost>,
    engine: Option<RunningEngine>,
    status: EngineStatus,
    conflicts: Vec<Conflict>,
    subscribers: Vec<(u64, SyncSender<ServerMessage>)>,
    /// Bagli arayuz surecleri: (istemci, Windows oturumu, tepsi mi).
    ui_clients: Vec<(u64, u32, bool)>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Controller {
    pub fn new(exe_dir: PathBuf, as_service: bool) -> (Arc<Self>, Receiver<EngineMsg>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let config = store::load_config();
        // Arayuze giden mesajlar (hata, uyari, yontem adi) ayarli dilde olur.
        shared::i18n::set(config.language);
        let learned = store::load_learned();
        let status = EngineStatus { method_name: config.active_method().name, dns_name: config.active_dns().display_name(), ..Default::default() };
        let ctrl = Arc::new_cyclic(|me| Self {
            inner: Mutex::new(Inner {
                config,
                learned,
                engine: None,
                status,
                conflicts: Vec::new(),
                subscribers: Vec::new(),
                ui_clients: Vec::new(),
            }),
            tx,
            exe_dir,
            as_service,
            me: me.clone(),
            updater: Updater::new(),
            update: Mutex::new(UpdateStatus::default()),
            conflict_watch: AtomicBool::new(false),
        });
        (ctrl, rx)
    }

    /// Motordan gelen bildirimleri isleyen arka plan is parcacigi.
    pub fn spawn_worker(self: &Arc<Self>, rx: Receiver<EngineMsg>) {
        let me = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("denetleyici".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                for msg in rx {
                    let Some(ctrl) = me.upgrade() else { break };
                    ctrl.on_engine_msg(msg);
                }
            })
            .expect("denetleyici is parcacigi");
    }

    fn on_engine_msg(&self, msg: EngineMsg) {
        let mut inner = self.inner.lock().unwrap();
        match msg {
            EngineMsg::Learned { host, reason } => {
                if inner.learned.iter().any(|h| h.host.eq_ignore_ascii_case(&host)) {
                    return;
                }
                let reason = match reason {
                    engine::LearnReason::Retransmit => "yanıtsız kaldı",
                    engine::LearnReason::Reset => "bağlantı sıfırlandı",
                };
                info!("Akıllı liste: {host} öğrenildi ({reason}).");
                let entry = LearnedHost { host, reason: reason.into(), at_ms: now_ms() };
                inner.learned.push(entry.clone());
                if let Err(e) = store::save_learned(&inner.learned) {
                    warn!("Öğrenilenler kaydedilemedi: {e}");
                }
                broadcast(&mut inner, Event::Learned { host: entry });
            }
            EngineMsg::Stopped { error } => {
                if let Some(engine) = inner.engine.take() {
                    engine.stop();
                }
                inner.status.state = EngineState::Error;
                inner.status.message = Some(error);
                inner.status.since_ms = None;
                let status = inner.status.clone();
                broadcast(&mut inner, Event::Engine { status });
            }
            EngineMsg::DnsHealth { ok } => {
                if inner.status.state != EngineState::On {
                    return;
                }
                inner.status.message =
                    (!ok).then(|| {
                        t!(
                            "Şifreli DNS'e ulaşılamıyor; geçici olarak sistem DNS'i kullanılıyor.",
                            "Can't reach encrypted DNS; using the system DNS for now."
                        )
                    });
                if ok {
                    info!("Şifreli DNS yeniden erişilebilir.");
                } else {
                    warn!("Şifreli DNS'e ulaşılamıyor; sistem DNS'ine düşüldü.");
                }
                let status = inner.status.clone();
                broadcast(&mut inner, Event::Engine { status });
            }
        }
    }

    pub fn config(&self) -> AppConfig {
        self.inner.lock().unwrap().config.clone()
    }

    pub fn handle(&self, req: Request, client: Option<(u64, &SyncSender<ServerMessage>)>) -> Response {
        match req {
            Request::Hello { client, protocol } => {
                info!("İstemci bağlandı: {client} (protokol {protocol}).");
                Response::Hello { service_version: shared::VERSION.into(), protocol: shared::ipc::PROTOCOL_VERSION }
            }
            Request::GetState => {
                // Arayuz acilirken liste tazelenir: kapanmis arac icin eski uyari gorunmesin.
                self.refresh_conflicts();
                Response::State { state: Box::new(self.snapshot()) }
            }
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
            Request::ForgetLearned { host } => {
                self.edit_learned(|list| list.retain(|h| !h.host.eq_ignore_ascii_case(&host)));
                Response::Ok
            }
            Request::ClearLearned => {
                self.edit_learned(Vec::clear);
                Response::Ok
            }
            Request::GetLogs => Response::Logs { lines: crate::log::lines() },
            Request::CheckUpdate { forced } => {
                if let Some(me) = self.me.upgrade() {
                    updater::request_check(&me, forced);
                }
                Response::Ok
            }
            Request::InstallUpdate => match self.me.upgrade().map(|me| updater::request_install(&me)) {
                Some(Err(message)) => Response::Error { message },
                _ => Response::Ok,
            },
            Request::FindMethod => Response::Error {
                message: t!("Bu özellik yalnızca Mac sürümünde var.", "This feature is only in the Mac version."),
            },
            Request::BenchDns => Response::DnsBench {
                results: crate::dns::bench::run_all()
                    .into_iter()
                    .map(|r| shared::ipc::BenchEntry {
                        id: r.id,
                        name: r.name,
                        ok: r.ok,
                        total: r.total,
                        median_ms: r.median_ms,
                        error: r.error,
                    })
                    .collect(),
            },
        }
    }

    pub fn snapshot(&self) -> StateSnapshot {
        let inner = self.inner.lock().unwrap();
        StateSnapshot {
            service_version: shared::VERSION.into(),
            engine: inner.status.clone(),
            config: inner.config.clone(),
            learned: inner.learned.clone(),
            conflicts: inner.conflicts.clone(),
            update: self.update_status(),
        }
    }

    pub fn updater(&self) -> &Updater {
        &self.updater
    }

    pub fn is_service(&self) -> bool {
        self.as_service
    }

    pub fn exe_dir(&self) -> &std::path::Path {
        &self.exe_dir
    }

    /// Istemci kendini tanittiginda: tepsi ve pencere hangi Windows oturumunda calisiyor
    /// (guncellemeden sonra ayni oturumda yeniden acilmalari icin).
    pub fn register_client(&self, id: u64, session: Option<u32>, name: &str) {
        let tray = match name {
            "tepsi" => true,
            "pencere" => false,
            _ => return,
        };
        let Some(session) = session else { return };
        let mut inner = self.inner.lock().unwrap();
        inner.ui_clients.retain(|(i, _, _)| *i != id);
        inner.ui_clients.push((id, session, tray));
    }

    /// Arayuzun acik oldugu oturumlar.
    pub fn ui_sessions(&self) -> Vec<UiSession> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<UiSession> = Vec::new();
        for &(_, session, tray) in &inner.ui_clients {
            let i = match out.iter().position(|s| s.session == session) {
                Some(i) => i,
                None => {
                    out.push(UiSession { session, ..Default::default() });
                    out.len() - 1
                }
            };
            if tray {
                out[i].tray = true;
            } else {
                out[i].window = true;
            }
        }
        out
    }

    pub fn update_status(&self) -> UpdateStatus {
        self.update.lock().unwrap().clone()
    }

    /// Guncelleme durumunu kaydeder ve abonelere duyurur. `finished`: denetim tamamlandi.
    pub fn set_update(&self, state: UpdateState, finished: bool) {
        let status = {
            let mut u = self.update.lock().unwrap();
            if finished && !matches!(state, UpdateState::Failed { .. }) {
                u.checked_at_ms = Some(now_ms());
            }
            if let UpdateState::Ready { version, .. } = &state {
                updater::remember_ready(version);
            }
            u.state = state;
            u.clone()
        };
        let mut inner = self.inner.lock().unwrap();
        broadcast(&mut inner, Event::Update { status });
    }

    fn stats(&self) -> StatsSnapshot {
        let inner = self.inner.lock().unwrap();
        inner.engine.as_ref().map(RunningEngine::stats).unwrap_or_default()
    }

    pub fn connect(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        self.start_engine(&mut inner)
    }

    fn start_engine(&self, inner: &mut Inner) -> Result<(), String> {
        if let Some(engine) = inner.engine.take() {
            engine.stop();
        }
        let method = inner.config.active_method();
        let dns = inner.config.active_dns();
        let (dns_mode, dns_setup) = match &dns {
            ActiveDns::Off => (DnsMode::Off, None),
            ActiveDns::Encrypted { url, bootstrap, .. } => (
                DnsMode::Intercept,
                Some(engine_loop::DnsSetup {
                    url: url.clone(),
                    bootstrap: bootstrap.clone(),
                    options: crate::dns::Options {
                        fallback_to_system: inner.config.dns.fallback_to_system,
                        skip_on_vpn: inner.config.dns.skip_on_vpn,
                    },
                }),
            ),
            ActiveDns::Redirect { v4, v6, .. } => (
                DnsMode::Redirect(Redirect { v4: v4.map(|(a, p)| (a.octets(), p)), v6: v6.map(|(a, p)| (a.octets(), p)) }),
                None,
            ),
        };
        let scope = inner.config.build_scope(inner.learned.iter().map(|h| h.host.clone()));
        let processor = Processor::new(method.config.clone(), dns_mode, scope);

        self.set_conflicts(inner, conflicts::scan());

        inner.status = EngineStatus {
            state: EngineState::Starting,
            message: None,
            since_ms: None,
            method_name: method.name.clone(),
            dns_name: dns.display_name(),
        };
        let status = inner.status.clone();
        broadcast(inner, Event::Engine { status });

        let result = match engine_loop::start(&self.exe_dir, processor, dns_setup, self.tx.clone()) {
            Ok(engine) => {
                info!("Motor başlatıldı: {} · {}", method.name, dns.display_name());
                inner.engine = Some(engine);
                inner.status.state = EngineState::On;
                inner.status.since_ms = Some(now_ms());
                Ok(())
            }
            Err(e) => {
                error!("Motor başlatılamadı: {e}");
                inner.status.state = EngineState::Error;
                inner.status.message = Some(e.clone());
                Err(e)
            }
        };
        let status = inner.status.clone();
        broadcast(inner, Event::Engine { status });
        result
    }

    pub fn disconnect(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(engine) = inner.engine.take() {
            engine.stop();
            info!("Motor durduruldu.");
        }
        inner.status.state = EngineState::Off;
        inner.status.message = None;
        inner.status.since_ms = None;
        let status = inner.status.clone();
        broadcast(&mut inner, Event::Engine { status });
    }

    fn set_config(&self, new: AppConfig) -> Result<(), String> {
        new.validate()?;
        let new = new.sanitized();
        shared::i18n::set(new.language);
        let mut inner = self.inner.lock().unwrap();
        let old = std::mem::replace(&mut inner.config, new.clone());
        store::save_config(&new).map_err(|e| t!("Ayarlar kaydedilemedi: {e}", "Couldn't save the settings: {e}"))?;

        if old.start_with_windows != new.start_with_windows && self.as_service {
            if let Err(e) = scm::set_auto_start(new.start_with_windows) {
                warn!("{e}");
            }
        }
        inner.status.method_name = new.active_method().name;
        inner.status.dns_name = new.active_dns().display_name();
        broadcast(&mut inner, Event::Config { config: Box::new(new.clone()) });

        if inner.engine.is_some() && old.engine_differs(&new) {
            info!("Ayarlar değişti; motor yeni ayarlarla yeniden başlatılıyor.");
            self.start_engine(&mut inner)?;
        } else {
            let status = inner.status.clone();
            broadcast(&mut inner, Event::Engine { status });
        }
        Ok(())
    }

    fn edit_learned(&self, f: impl FnOnce(&mut Vec<LearnedHost>)) {
        let mut inner = self.inner.lock().unwrap();
        f(&mut inner.learned);
        if let Err(e) = store::save_learned(&inner.learned) {
            warn!("Öğrenilenler kaydedilemedi: {e}");
        }
        // Calisan motor eski listeyle devam etmesin.
        if inner.engine.is_some() {
            let _ = self.start_engine(&mut inner);
        }
    }

    /// Cakisan araclari yeniden tarar.
    pub fn refresh_conflicts(&self) {
        let found = conflicts::scan();
        let mut inner = self.inner.lock().unwrap();
        self.set_conflicts(&mut inner, found);
    }

    /// Yeni cakisma listesini kaydeder, degistiyse duyurur; liste doluysa izleyiciyi baslatir.
    fn set_conflicts(&self, inner: &mut Inner, found: Vec<Conflict>) {
        apply_conflicts(inner, found);
        if inner.conflicts.is_empty() || self.conflict_watch.swap(true, Ordering::SeqCst) {
            return;
        }
        // Liste dolu oldugu surece izlenir: cakisan arac kapaninca uyari motor yeniden baslamadan
        // kalkar. Bayrak kilit altinda birakilir; izleyici cikarken gelen yeni cakisma kacmaz.
        let me = self.me.clone();
        let spawned = std::thread::Builder::new().name("cakisma".into()).stack_size(64 * 1024).spawn(move || {
            loop {
                std::thread::sleep(CONFLICT_POLL);
                let Some(ctrl) = me.upgrade() else { return };
                let found = conflicts::scan();
                let mut inner = ctrl.inner.lock().unwrap();
                apply_conflicts(&mut inner, found);
                if inner.conflicts.is_empty() {
                    ctrl.conflict_watch.store(false, Ordering::SeqCst);
                    return;
                }
            }
        });
        if spawned.is_err() {
            self.conflict_watch.store(false, Ordering::SeqCst);
        }
    }

    /// Istemci ayrilinca aboneligini hemen sil (yazici is parcacigi beklemesin).
    pub fn unsubscribe(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.subscribers.retain(|(i, _)| *i != id);
        inner.ui_clients.retain(|(i, _, _)| *i != id);
    }

    pub fn shutdown(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(engine) = inner.engine.take() {
            engine.stop();
        }
        inner.subscribers.clear();
    }
}

/// Liste degistiyse kaydeder, gunluge yazar ve abonelere duyurur.
fn apply_conflicts(inner: &mut Inner, found: Vec<Conflict>) {
    if found == inner.conflicts {
        return;
    }
    if found.is_empty() {
        info!("Çakışan araç kapandı; uyarı kaldırıldı.");
    } else {
        let names: Vec<_> = found.iter().map(|c| c.process.as_str()).collect();
        warn!("Çakışabilecek araçlar çalışıyor: {}", names.join(", "));
    }
    inner.conflicts = found;
    let conflicts = inner.conflicts.clone();
    broadcast(inner, Event::Conflicts { conflicts });
}

fn broadcast(inner: &mut Inner, event: Event) {
    let msg = ServerMessage::Event { event };
    // Yavas ya da kopmus istemci denetleyiciyi bekletmesin: kuyruk doluysa atilir.
    inner.subscribers.retain(|(_, s)| !matches!(s.try_send(msg.clone()), Err(TrySendError::Disconnected(_))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Eski surum kapaninca "iki DPI araci" uyarisi motor yeniden baslamadan kalkar. Gercek bir
    /// surec kullanilir: ping.exe "goodbyedpi-ui.exe" adiyla calistirilip sonra kapatilir.
    #[test]
    fn conflict_warning_clears_when_tool_exits() {
        let dir = std::env::temp_dir().join(format!("gdpi3-cakisma-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("goodbyedpi-ui.exe");
        std::fs::copy(r"C:\Windows\System32\PING.EXE", &fake).unwrap();
        let mut child = std::process::Command::new(&fake)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();

        let (ctrl, _rx) = Controller::new(dir.clone(), false);
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        ctrl.handle(Request::Subscribe, Some((1, &tx)));
        let Response::State { state } = ctrl.handle(Request::GetState, None) else { panic!("durum gelmedi") };
        assert!(state.conflicts.iter().any(|c| c.pid == pid), "calisan arac uyarida olmali: {:?}", state.conflicts);

        let _ = child.kill();
        let _ = child.wait();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut cleared = false;
        while let Ok(msg) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            if let ServerMessage::Event { event: Event::Conflicts { conflicts } } = msg {
                if conflicts.iter().all(|c| c.pid != pid) {
                    cleared = true;
                    break;
                }
            }
        }
        assert!(cleared, "arac kapaninca bos liste duyurulmali");
        assert!(ctrl.snapshot().conflicts.iter().all(|c| c.pid != pid));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
