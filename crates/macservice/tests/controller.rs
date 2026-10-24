//! Mac hizmetinin denetleyicisi: baglan/kes, ayar degisikligi, PAC ve sistem ayari. Sistem
//! ayari sahte (cagrilari kaydeder); proxy gercek (127.0.0.1, rastgele port).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};

use macservice::Controller;
use macservice::sysproxy::SystemProxy;
use shared::config::{AppConfig, DnsMode};
use shared::i18n::Language;
use shared::ipc::{EngineState, Event, Request, Response, ServerMessage};

#[derive(Clone, Default)]
struct FakeSys {
    calls: Arc<Mutex<Vec<String>>>,
    fail: Arc<Mutex<bool>>,
}

impl SystemProxy for FakeSys {
    fn enable(&mut self, url: &str) -> Result<(), String> {
        if *self.fail.lock().unwrap() {
            return Err("yetki yok".into());
        }
        self.calls.lock().unwrap().push(format!("enable {url}"));
        Ok(())
    }
    fn disable(&mut self) -> Result<(), String> {
        self.calls.lock().unwrap().push("disable".into());
        Ok(())
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dpi-macservice-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn setup(name: &str) -> (Arc<Controller>, FakeSys, PathBuf) {
    let sys = FakeSys::default();
    let dir = temp_dir(name);
    // Metin karsilastirmalari makinenin dilinden bagimsiz olsun.
    let cfg = AppConfig { language: Language::Turkish, ..AppConfig::default() }.sanitized();
    shared::i18n::set(Language::Turkish);
    (Controller::new(cfg, Box::new(sys.clone()), dir.clone(), 0), sys, dir)
}

fn pac(ctrl: &Controller) -> String {
    let mut c = TcpStream::connect(ctrl.proxy_addr().unwrap()).unwrap();
    c.write_all(b"GET /proxy.pac HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut out = String::new();
    c.read_to_string(&mut out).unwrap();
    out
}

fn calls(sys: &FakeSys) -> Vec<String> {
    sys.calls.lock().unwrap().clone()
}

#[test]
fn connect_points_the_system_at_the_pac_and_disconnect_restores() {
    let (ctrl, sys, _) = setup("baglan");
    let (tx, rx) = sync_channel::<ServerMessage>(64);
    assert_eq!(ctrl.handle(Request::Subscribe, Some((1, &tx))), Response::Ok);

    assert_eq!(ctrl.handle(Request::Connect, None), Response::Ok);
    let addr = ctrl.proxy_addr().expect("proxy calisiyor");
    assert_eq!(calls(&sys), [format!("enable http://{addr}/proxy.pac?v=1")]);
    let status = ctrl.status();
    assert_eq!(status.state, EngineState::On);
    assert_eq!(status.method_name, "TLS kaydı bölme");
    let script = pac(&ctrl);
    assert!(script.contains("\"discord.com\": 1") && script.contains(&format!("PROXY {addr}; DIRECT")), "{script}");
    assert!(matches!(rx.try_recv(), Ok(ServerMessage::Event { event: Event::Engine { .. } })));

    ctrl.handle(Request::Disconnect, None);
    assert_eq!(calls(&sys).last().unwrap(), "disable");
    assert!(ctrl.proxy_addr().is_none());
    assert_eq!(ctrl.status().state, EngineState::Off);
}

#[test]
fn config_changes_apply_to_the_running_connection() {
    let (ctrl, sys, dir) = setup("ayar");
    ctrl.connect().unwrap();
    let addr = ctrl.proxy_addr().unwrap();

    // Yontem: proxy yerinde degisir, sistem ayarina dokunulmaz.
    let mut cfg = ctrl.config();
    cfg.proxy_method = "disorder".into();
    ctrl.set_config(cfg).unwrap();
    assert_eq!(ctrl.status().method_name, "Ters sıra");
    assert_eq!(calls(&sys).len(), 1);
    assert_eq!(ctrl.proxy_addr(), Some(addr));
    let saved: AppConfig = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved.proxy_method, "disorder");

    // Liste: PAC yenilenir ve yeni surum adresi sisteme yazilir.
    let mut cfg = ctrl.config();
    cfg.scope.targets.push("ornek-site.com".into());
    ctrl.set_config(cfg).unwrap();
    assert_eq!(calls(&sys).last().unwrap(), &format!("enable http://{addr}/proxy.pac?v=2"));
    assert!(pac(&ctrl).contains("\"ornek-site.com\": 1"));

    // DNS: cozucu proxy'yle kurulur, baglanti yeniden acilir.
    let mut cfg = ctrl.config();
    cfg.dns.mode = DnsMode::Off;
    ctrl.set_config(cfg).unwrap();
    assert_eq!(ctrl.status().state, EngineState::On);
    assert_eq!(ctrl.status().dns_name, "Sistem DNS'i");
    assert!(calls(&sys).last().unwrap().ends_with("?v=3"), "{:?}", calls(&sys));

    // Bilinmeyen yontem reddedilir, bos kimlik varsayilana duser.
    let mut cfg = ctrl.config();
    cfg.proxy_method = "yok".into();
    assert!(ctrl.set_config(cfg).is_err());
    let mut cfg = ctrl.config();
    cfg.proxy_method = String::new();
    ctrl.set_config(cfg).unwrap();
    assert_eq!(ctrl.config().proxy_method, "tlsrec");
    ctrl.disconnect();
}

#[test]
fn failed_system_setting_leaves_nothing_running() {
    let (ctrl, sys, _) = setup("hata");
    *sys.fail.lock().unwrap() = true;
    let r = ctrl.handle(Request::Connect, None);
    let Response::Error { message } = r else { panic!("{r:?}") };
    assert!(message.contains("yetki yok"), "{message}");
    assert_eq!(ctrl.status().state, EngineState::Error);
    assert!(ctrl.proxy_addr().is_none(), "proxy acik kalmamali");
    assert_eq!(calls(&sys), ["disable"], "yarim kalan ayar geri alinir");
}

#[test]
fn snapshot_has_no_learning_or_conflicts() {
    let (ctrl, _, _) = setup("durum");
    let Response::State { state } = ctrl.handle(Request::GetState, None) else { panic!() };
    assert!(state.learned.is_empty() && state.conflicts.is_empty());
    assert_eq!(state.engine.state, EngineState::Off);
    assert_eq!(ctrl.handle(Request::ClearLearned, None), Response::Ok);
}
