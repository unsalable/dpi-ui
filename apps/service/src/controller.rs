//! Servisin merkezi durumu: ayar, motor, ogrenilenler ve abone istemciler.
//! Tum degisiklikler buradan gecer; arayuzler yalnizca istek gonderir.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use engine::{DnsMode, Processor, Redirect};
use shared::config::{ActiveDns, AppConfig};
use shared::ipc::{EngineState, EngineStatus, Event, LearnedHost, Request, Response, ServerMessage, StateSnapshot, StatsSnapshot};

use crate::engine_loop::{self, EngineMsg, RunningEngine};
use crate::{conflicts, error, info, scm, store, warn};

pub struct Controller {
    inner: Mutex<Inner>,
    tx: Sender<EngineMsg>,
    exe_dir: PathBuf,
    as_service: bool,
}

struct Inner {
    config: AppConfig,
    learned: Vec<LearnedHost>,
    engine: Option<RunningEngine>,
    status: EngineStatus,
    conflicts: Vec<shared::ipc::Conflict>,
    subscribers: Vec<(u64, SyncSender<ServerMessage>)>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Controller {
    pub fn new(exe_dir: PathBuf, as_service: bool) -> (Arc<Self>, Receiver<EngineMsg>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let config = store::load_config();
        let learned = store::load_learned();
        let status = EngineStatus { method_name: config.active_method().name, dns_name: config.active_dns().display_name(), ..Default::default() };
        let ctrl = Arc::new(Self {
            inner: Mutex::new(Inner { config, learned, engine: None, status, conflicts: Vec::new(), subscribers: Vec::new() }),
            tx,
            exe_dir,
            as_service,
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
                    (!ok).then(|| "Şifreli DNS'e ulaşılamıyor; geçici olarak sistem DNS'i kullanılıyor.".to_owned());
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
            Request::ForgetLearned { host } => {
                self.edit_learned(|list| list.retain(|h| !h.host.eq_ignore_ascii_case(&host)));
                Response::Ok
            }
            Request::ClearLearned => {
                self.edit_learned(Vec::clear);
                Response::Ok
            }
            Request::GetLogs => Response::Logs { lines: crate::log::lines() },
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
        }
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

        inner.conflicts = conflicts::scan();
        if !inner.conflicts.is_empty() {
            let names: Vec<_> = inner.conflicts.iter().map(|c| c.process.as_str()).collect();
            warn!("Çakışabilecek araçlar çalışıyor: {}", names.join(", "));
            let conflicts = inner.conflicts.clone();
            broadcast(inner, Event::Conflicts { conflicts });
        }

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
        let mut inner = self.inner.lock().unwrap();
        let old = std::mem::replace(&mut inner.config, new.clone());
        store::save_config(&new).map_err(|e| format!("Ayarlar kaydedilemedi: {e}"))?;

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

    /// Istemci ayrilinca aboneligini hemen sil (yazici is parcacigi beklemesin).
    pub fn unsubscribe(&self, id: u64) {
        self.inner.lock().unwrap().subscribers.retain(|(i, _)| *i != id);
    }

    pub fn shutdown(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(engine) = inner.engine.take() {
            engine.stop();
        }
        inner.subscribers.clear();
    }
}

fn broadcast(inner: &mut Inner, event: Event) {
    let msg = ServerMessage::Event { event };
    // Yavas ya da kopmus istemci denetleyiciyi bekletmesin: kuyruk doluysa atilir.
    inner.subscribers.retain(|(_, s)| !matches!(s.try_send(msg.clone()), Err(TrySendError::Disconnected(_))));
}
