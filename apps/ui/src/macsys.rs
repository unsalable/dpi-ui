//! Mac platform katmani. Pencere kodu bunu Windows'taki gibi `winsys` adiyla cagirir: tema,
//! oturum acilisinda baslatma (LaunchAgent), animasyon tercihi, adres/dosya acma.

use std::path::{Path, PathBuf};
use std::process::Command;

use slint::ComponentHandle;
use slint::winit_030::{WinitWindowAccessor, winit};

use crate::AppWindow;

const AGENT_LABEL: &str = "com.dpi-ui.app";

/// Sistem acik gorunumde mi? `AppleInterfaceStyle` yalnizca koyu modda ("Dark") yazilidir.
pub fn system_prefers_light() -> bool {
    Command::new("/usr/bin/defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().eq_ignore_ascii_case("dark"))
        .unwrap_or(false)
}

/// Pencere cubugu ve trafik isiklari secili temaya uysun.
pub fn set_window_theme(app: &AppWindow, dark: bool) {
    app.window().with_winit_window(|w| {
        w.set_theme(Some(if dark { winit::window::Theme::Dark } else { winit::window::Theme::Light }));
    });
}

/// Erisilebilirlikte "Hareketi azalt" aciksa animasyonlar kapali.
pub fn animations_enabled() -> bool {
    Command::new("/usr/bin/defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() != "1")
        .unwrap_or(true)
}

/// Motor uygulamanin icinde calisir; basladiysa zaten acik, baslamadiysa uygulama yeniden acilmali.
pub fn start_service() -> Result<(), String> {
    Err(shared::t!(
        "Motor hizmeti başlatılamadı. Uygulamayı kapatıp yeniden aç.",
        "The engine service couldn't start. Quit the app and open it again."
    ))
}

pub fn shell_open(target: &str) {
    let _ = Command::new("/usr/bin/open").arg(target).spawn();
}

/// Uygulama paketi (`.../DPI UI.app`); paket disindan calisiyorsa `None`.
fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

fn agent_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library").join("LaunchAgents").join(format!("{AGENT_LABEL}.plist")))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Oturum acilinca uygulamayi arka planda (yalnizca menu cubugunda) baslatan LaunchAgent.
fn agent_plist() -> Option<String> {
    let args: Vec<String> = match bundle_path() {
        // LaunchServices ile: Dock ve izinler normal acilistaki gibi; -g one getirmez.
        Some(app) => vec!["/usr/bin/open".into(), "-g".into(), "-a".into(), app.display().to_string(), "--args".into(), "--background".into()],
        None => vec![std::env::current_exe().ok()?.display().to_string(), "--background".into()],
    };
    let items: String = args.iter().map(|a| format!("\t\t<string>{}</string>\n", xml_escape(a))).collect();
    Some(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{AGENT_LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n{items}\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         </dict>\n</plist>\n"
    ))
}

/// Oturum acilisinda baslatmayi acar/kapatir. Basarisizsa false.
pub fn set_autorun(on: bool) -> bool {
    let Some(file) = agent_file() else { return false };
    if !on {
        return std::fs::remove_file(&file).is_ok() || !file.exists();
    }
    let Some(plist) = agent_plist() else { return false };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&file, plist).is_ok()
}

/// Uygulama tasindiysa (orn. Indirilenler'den Uygulamalar'a) kaydi yeni yola gore yeniler.
pub fn repair_autorun(on: bool) {
    let Some(file) = agent_file() else { return };
    let current = std::fs::read_to_string(&file).ok();
    match (on, current) {
        (true, Some(text)) if agent_plist().as_deref() == Some(text.as_str()) => {}
        (true, _) => {
            set_autorun(true);
        }
        (false, Some(_)) => {
            set_autorun(false);
        }
        (false, None) => {}
    }
}
