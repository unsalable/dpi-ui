//! Mac'e ozgu uygulama parcalari:
//! - Pencere: macOS'un kendi pencere cubugu (trafik isiklari) seffaf ve icerigin ustunde.
//! - Menu cubugu simgesi: durum, Baglan/Kes, pencereyi ac, cik (Windows'taki tepsinin karsiligi).
//! - Dock simgesine tiklaninca ya da uygulama yeniden acilinca gizli pencere geri gelir.
//! - Cmd+Q / oturum kapatma: sistem proxy ayari eski haline doner.
//! - Paketleme icin uygulama simgesi PNG'leri (.icns'e `iconutil` cevirir).

use std::cell::RefCell;
use std::path::Path;
use std::ptr::NonNull;
use std::time::Duration;

use block2::RcBlock;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::iconart::{self, Mark};
use crate::PowerState;

/// Yerli pencere cubugu: baslik gizli, cubuk seffaf, icerik pencerenin tamamini kaplar
/// (arayuz ustte 28 px pay birakir). Slint penceresi olusmadan secilmeli.
pub fn select_backend() -> Result<(), slint::PlatformError> {
    use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attrs| attrs.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true))
        .select()
}

struct MenuBar {
    tray: TrayIcon,
    status: MenuItem,
    toggle: MenuItem,
    open: MenuItem,
    quit: MenuItem,
    shown: Option<PowerState>,
}

thread_local! {
    static BAR: RefCell<Option<MenuBar>> = const { RefCell::new(None) };
    static LAST: RefCell<Option<(PowerState, String)>> = const { RefCell::new(None) };
}

/// Menu cubugu simgesi: sablon (tek renk) resim; macOS acik/koyu cubuga gore boyar.
/// Kapaliyken soluk, bagliyken tam.
fn tray_icon(power: PowerState) -> Option<Icon> {
    const SIZE: u32 = 36; // 18 pt, Retina
    let alpha = if matches!(power, PowerState::On) { 1.0 } else { 0.45 };
    let bgra = iconart::render_bgra(SIZE, Mark::Off);
    let rgba: Vec<u8> = bgra.chunks_exact(4).flat_map(|p| [0, 0, 0, (p[3] as f32 * alpha).round() as u8]).collect();
    Icon::from_rgba(rgba, SIZE, SIZE).ok()
}

/// Olay dongusu basladiktan sonra menu cubugu simgesi ve Dock gozlemcisi kurulur (NSApplication
/// hazir olmadan durum ogesi olusturulamaz).
pub fn install() {
    slint::Timer::single_shot(Duration::ZERO, || {
        build_menubar();
        observe(unsafe { objc2_app_kit::NSApplicationDidBecomeActiveNotification }, || crate::window::show_window());
    });
}

fn build_menubar() {
    let en = shared::i18n::english();
    let status = MenuItem::new(if en { "Off" } else { "Kapalı" }, false, None);
    let toggle = MenuItem::new(if en { "Connect" } else { "Bağlan" }, true, None);
    let open = MenuItem::new(if en { "Open DPI UI" } else { "DPI UI'yi aç" }, true, None);
    let quit = MenuItem::new(if en { "Quit DPI UI" } else { "DPI UI'den çık" }, true, None);
    let menu = Menu::new();
    let sep1 = PredefinedMenuItem::separator();
    let sep2 = PredefinedMenuItem::separator();
    if menu.append_items(&[&status, &toggle, &sep1, &open, &sep2, &quit]).is_err() {
        return;
    }
    let Ok(tray) = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon_templated(tray_icon(PowerState::Off).expect("simge"))
        .with_tooltip("DPI UI")
        .build()
    else {
        return;
    };
    let ids = (toggle.id().clone(), open.id().clone(), quit.id().clone());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let (toggle, open, quit) = ids.clone();
        let id: MenuId = e.id;
        let _ = slint::invoke_from_event_loop(move || {
            if id == toggle {
                crate::window::menu_power_toggle();
            } else if id == open {
                crate::window::show_window();
            } else if id == quit {
                crate::window::quit_app();
            }
        });
    }));
    BAR.with(|b| *b.borrow_mut() = Some(MenuBar { tray, status, toggle, open, quit, shown: None }));
    // Simge kurulmadan once gelen durum.
    if let Some((power, title)) = LAST.with(|l| l.borrow().clone()) {
        show_state(power, &title);
    }
}

/// Menu cubugunu motorun durumuna esitler (pencere kodu her durum degisiminde cagirir).
pub fn show_state(power: PowerState, title: &str) {
    LAST.with(|l| *l.borrow_mut() = Some((power, title.to_owned())));
    BAR.with(|b| {
        let mut b = b.borrow_mut();
        let Some(bar) = b.as_mut() else { return };
        let en = shared::i18n::english();
        bar.status.set_text(title);
        let connected = matches!(power, PowerState::On | PowerState::Connecting);
        bar.toggle.set_text(match (connected, en) {
            (true, false) => "Bağlantıyı kes",
            (true, true) => "Disconnect",
            (false, false) => "Bağlan",
            (false, true) => "Connect",
        });
        bar.open.set_text(if en { "Open DPI UI" } else { "DPI UI'yi aç" });
        bar.quit.set_text(if en { "Quit DPI UI" } else { "DPI UI'den çık" });
        let solid = matches!(power, PowerState::On);
        if bar.shown.map(|p| matches!(p, PowerState::On)) != Some(solid) {
            let _ = bar.tray.set_icon_templated(tray_icon(power));
            bar.shown = Some(power);
        }
        let _ = bar.tray.set_tooltip(Some(format!("DPI UI · {title}")));
    });
}

/// Pencere kapaninca uygulama da gizlenir (Cmd+H gibi). Etkin uygulamanin Dock simgesine
/// tiklamak "etkinlesti" bildirimi uretmez; gizli uygulamaya tiklamak uretir ve pencere geri gelir.
pub fn hide_app() {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).hide(None);
    }
}

/// Menu cubugundan "ac": gizli uygulama once gorunur olmali.
pub fn unhide_app() {
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).unhide(None);
    }
}

/// Bildirim merkezine gozlemci ekler (uygulama omru boyunca kalir).
fn observe(name: &'static objc2_foundation::NSNotificationName, f: impl Fn() + 'static) {
    let block = RcBlock::new(move |_: NonNull<NSNotification>| f());
    let center = NSNotificationCenter::defaultCenter();
    let queue = NSOperationQueue::mainQueue();
    // Gozlemci kaydi uygulama kapanana kadar tutulur.
    let token = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, Some(&queue), &block) };
    std::mem::forget(token);
}

/// Uygulama kapanmadan hemen once (Cmd+Q, oturum kapatma, yeniden baslatma) cagrilir.
pub fn on_terminate(f: impl Fn() + 'static) {
    observe(unsafe { objc2_app_kit::NSApplicationWillTerminateNotification }, f);
}

/// `iconutil -c icns` icin `.iconset` klasoru. macOS simge izgarasina uygun: kare 1024'te
/// 824 px icerik, cevresi saydam.
pub fn write_iconset(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (base, scale) in [(16u32, 1u32), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2), (256, 1), (256, 2), (512, 1), (512, 2)] {
        let size = base * scale;
        let inner = ((size as f32) * 0.875).round() as u32;
        let bgra = iconart::render_bgra(inner, Mark::App);
        let off = (size - inner) / 2;
        let mut rgba = vec![0u8; (size * size * 4) as usize];
        for y in 0..inner {
            for x in 0..inner {
                let s = ((y * inner + x) * 4) as usize;
                let d = (((y + off) * size + x + off) * 4) as usize;
                rgba[d..d + 4].copy_from_slice(&[bgra[s + 2], bgra[s + 1], bgra[s], bgra[s + 3]]);
            }
        }
        let name = if scale == 1 { format!("icon_{base}x{base}.png") } else { format!("icon_{base}x{base}@2x.png") };
        let file = std::fs::File::create(dir.join(name))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(std::io::Error::other)?;
        w.write_image_data(&rgba).map_err(std::io::Error::other)?;
    }
    Ok(())
}
