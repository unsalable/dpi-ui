//! DPI UI motor hizmeti.
//!
//! Kullanim:
//!   GoodbyeDPI3-Service.exe --service      (Windows hizmet yoneticisi baslatir)
//!   GoodbyeDPI3-Service.exe --console      (gelistirme: on planda calistir, yonetici)
//!   GoodbyeDPI3-Service.exe --install [--manual]
//!   GoodbyeDPI3-Service.exe --uninstall

mod conflicts;
mod controller;
mod dns;
mod engine_loop;
mod https;
mod launch;
mod log;
mod pipe;
mod probe;
mod scm;
mod store;
mod updater;

use std::ffi::OsString;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use controller::Controller;
use shared::config::AppConfig;
use shared::i18n::Language;

struct App {
    ctrl: Arc<Controller>,
    pipe: pipe::PipeServer,
}

impl App {
    fn start(as_service: bool) -> App {
        store::ensure_data_dir();
        log::init(!as_service, Some(shared::paths::log_dir().join("service.log")));
        info!("{} motoru {} başlıyor ({}).", shared::PRODUCT_NAME, shared::VERSION, if as_service { "hizmet" } else { "konsol" });

        let exe_dir = shared::paths::exe_dir().unwrap_or_default();
        let (ctrl, rx) = Controller::new(exe_dir, as_service);
        ctrl.spawn_worker(rx);
        if ctrl.config().auto_connect {
            let _ = ctrl.connect();
        }
        updater::spawn_schedule(&ctrl);
        let pipe = pipe::start(ctrl.clone());
        // Sessiz guncellemeden sonra: kapanan arayuzleri kullanicinin oturumunda yeniden ac.
        if as_service {
            updater::after_start(&ctrl);
        }
        App { ctrl, pipe }
    }

    fn stop(self) {
        info!("Motor hizmeti kapanıyor.");
        self.pipe.stop();
        self.ctrl.shutdown();
    }
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_service() {
        error!("Hizmet hatası: {e}");
    }
}

fn run_service() -> windows_service::Result<()> {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handler = move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status = service_control_handler::register(shared::SERVICE_NAME, handler)?;
    let report = |state, accept, wait| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: wait,
        process_id: None,
    };

    status.set_service_status(report(ServiceState::StartPending, ServiceControlAccept::empty(), Duration::from_secs(10)))?;
    let app = App::start(true);
    status.set_service_status(report(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        Duration::default(),
    ))?;

    let _ = stop_rx.recv();
    status.set_service_status(report(ServiceState::StopPending, ServiceControlAccept::empty(), Duration::from_secs(10)))?;
    app.stop();
    status.set_service_status(report(ServiceState::Stopped, ServiceControlAccept::empty(), Duration::default()))?;
    Ok(())
}

fn run_console() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    static STOP: std::sync::OnceLock<mpsc::Sender<()>> = std::sync::OnceLock::new();
    unsafe extern "system" fn on_ctrl(_: u32) -> i32 {
        if let Some(tx) = STOP.get() {
            let _ = tx.send(());
        }
        1
    }
    let (tx, rx) = mpsc::channel();
    let _ = STOP.set(tx);
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };

    let app = App::start(false);
    println!("Çalışıyor. Durdurmak için Ctrl+C.");
    let _ = rx.recv();
    app.stop();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a.eq_ignore_ascii_case(flag));

    let code = if has("--service") {
        match service_dispatcher::start(shared::SERVICE_NAME, ffi_service_main) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("Hizmet yöneticisi tarafından başlatılmadı: {e}");
                1
            }
        }
    } else if has("--console") {
        run_console();
        0
    } else if has("--install") {
        // Ilk kurulum: uygulamanin dili kurulumda secilen dil olur (--lang tr|en). Ayar dosyasi
        // zaten varsa (guncelleme) kullanicinin secimine dokunulmaz.
        let lang = args.iter().position(|a| a == "--lang").and_then(|i| args.get(i + 1));
        if let Some(lang) = lang.filter(|_| !shared::paths::config_file().exists()) {
            store::ensure_data_dir();
            let language = if lang.eq_ignore_ascii_case("en") { Language::English } else { Language::Turkish };
            let _ = store::save_config(&AppConfig { language, ..AppConfig::default() });
        }
        // Hizmetin Windows'taki adi ve aciklamasi ayarli dilde.
        shared::i18n::set(store::load_config().language);
        let exe = std::env::current_exe().unwrap_or_default();
        match scm::install(&exe, !has("--manual")) {
            Ok(()) => {
                println!("Hizmet kuruldu: {}", shared::SERVICE_NAME);
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        }
    } else if has("--probe") {
        let method = args.iter().position(|a| a == "--method").and_then(|i| args.get(i + 1)).map(String::as_str);
        log::init(true, None);
        probe::run(method)
    } else if has("--dns-bench") {
        for r in dns::bench::run_all() {
            let ms = |v: Option<u32>| v.map_or("-".to_owned(), |m| format!("{m} ms"));
            println!(
                "{:<16} {}/{}  ilk {:>7}  medyan {:>7}  {}",
                r.name,
                r.ok,
                r.total,
                ms(r.first_ms),
                ms(r.median_ms),
                r.error.unwrap_or_default()
            );
        }
        0
    } else if has("--check-update") {
        // Yonetici gerekmez: denetler, indirir ve dogrular, kurmaz. Test icin --as-version ile
        // eski bir surumdeymis gibi davranilabilir; GDPI3_DATA_DIR indirme yerini degistirir.
        let current = args
            .iter()
            .position(|a| a == "--as-version")
            .and_then(|i| args.get(i + 1))
            .map_or(shared::VERSION, String::as_str);
        let Some(current) = shared::update::Version::parse(current) else {
            eprintln!("sürüm biçimi geçersiz");
            std::process::exit(2);
        };
        println!("Şu anki sürüm: {current}");
        match updater::check_and_download(&current, has("--beta"), &mut |s| println!("  {s:?}")) {
            Ok(state) => {
                println!("Sonuç: {state:?}");
                if matches!(state, shared::update::UpdateState::Ready { .. }) {
                    println!("Paket: {}", shared::paths::update_installer().display());
                }
                0
            }
            Err(e) => {
                eprintln!("Hata: {e}");
                1
            }
        }
    } else if has("--uninstall") {
        match scm::uninstall() {
            Ok(()) => {
                println!("Hizmet kaldırıldı.");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        }
    } else {
        println!(
            "{} motoru {}\n\n  --console     on planda calistir (yonetici)\n  --probe       canli test: siteleri motor kapali/acik dener (yonetici) [--method kimlik]\n  --dns-bench   sifreli DNS saglayicilarini olc\n  --check-update  guncellemeyi denetle, indir, dogrula; kurmaz [--as-version 3.0.0] [--beta]\n  --install     hizmeti kur [--manual]\n  --uninstall   hizmeti kaldir",
            shared::PRODUCT_NAME,
            shared::VERSION
        );
        0
    };
    std::process::exit(code);
}
