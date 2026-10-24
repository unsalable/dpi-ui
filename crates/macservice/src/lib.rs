//! Mac'te motor hizmeti. Windows'ta motor ayri bir Windows hizmetidir; Mac'te yonetici izni
//! gerektirmedigi icin uygulamanin icinde calisir. Arayuz onunla yine IPC uzerinden, Windows
//! servisiyle ayni mesajlarla konusur (Unix soketi), boylece pencere kodu ayni kalir.

pub mod bench;
pub mod controller;
#[macro_use]
pub mod log;
pub mod search;
#[cfg(unix)]
pub mod server;
pub mod store;
pub mod sysproxy;
pub mod updates;

use std::path::PathBuf;
use std::sync::Arc;

pub use controller::Controller;

/// Arayuzun baglandigi soket.
pub fn socket_path() -> PathBuf {
    shared::paths::service_data_dir().join("ipc.sock")
}

pub struct Service {
    ctrl: Arc<Controller>,
    #[cfg(unix)]
    server: Option<server::Server>,
}

/// Hizmeti baslatir: onceki calismadan kalan sistem ayarini geri alir, ayari okur, soketi acar
/// ve ayar istiyorsa baglanir.
pub fn start() -> std::io::Result<Service> {
    let dir = shared::paths::service_data_dir();
    std::fs::create_dir_all(&dir)?;
    log::init(&dir.join("logs"));
    info!("DPI UI {} (Mac) başlıyor.", shared::VERSION);

    let config = store::load_config(&dir);
    shared::i18n::set(config.language);
    let auto_connect = config.auto_connect;

    #[cfg(target_os = "macos")]
    let sys: Box<dyn sysproxy::SystemProxy> = {
        let mut ns = sysproxy::Networksetup::new(dir.join("sysproxy.json"));
        if let Some(note) = ns.restore_stale() {
            warn!("{note}");
        }
        Box::new(ns)
    };
    #[cfg(not(target_os = "macos"))]
    let sys: Box<dyn sysproxy::SystemProxy> = Box::new(sysproxy::Untouched);

    let ctrl = Controller::new(config, sys, dir, proxy::DEFAULT_PORT);
    #[cfg(unix)]
    let server = Some(server::start(ctrl.clone(), &socket_path())?);
    if auto_connect {
        let _ = ctrl.connect();
    }
    // Saatte bir bakar; denetim en sik 6 saatte bir aga cikar (arayuz acilinca da denetler).
    let weak = Arc::downgrade(&ctrl);
    std::thread::Builder::new().name("guncelleme".into()).stack_size(128 * 1024).spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            match weak.upgrade() {
                Some(c) => c.check_update(false),
                None => return,
            }
        }
    })?;
    Ok(Service {
        ctrl,
        #[cfg(unix)]
        server,
    })
}

impl Service {
    pub fn controller(&self) -> &Arc<Controller> {
        &self.ctrl
    }

    /// Uygulama kapanirken: sistem proxy ayari eski haline doner, soket kapanir.
    pub fn shutdown(self) {
        self.ctrl.disconnect();
        #[cfg(unix)]
        if let Some(s) = self.server {
            s.stop();
        }
        info!("Kapandı.");
    }
}
