#![windows_subsystem = "windows"]

use std::time::{Duration, Instant};

use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ComponentHandle, Timer, TimerMode};

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;

    let connected_at = std::rc::Rc::new(std::cell::Cell::new(None::<Instant>));

    app.on_power_clicked({
        let weak = app.as_weak();
        let connected_at = connected_at.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            match app.get_power_state() {
                PowerState::Off | PowerState::Error => {
                    app.set_power_state(PowerState::Connecting);
                    let weak = app.as_weak();
                    let connected_at = connected_at.clone();
                    // Deneme: gercek motor yerine 1.4 sn sonra "bagli".
                    Timer::single_shot(Duration::from_millis(1400), move || {
                        let Some(app) = weak.upgrade() else { return };
                        if app.get_power_state() == PowerState::Connecting {
                            connected_at.set(Some(Instant::now()));
                            app.set_uptime("Az önce bağlandı".into());
                            app.set_power_state(PowerState::On);
                        }
                    });
                }
                _ => {
                    connected_at.set(None);
                    app.set_power_state(PowerState::Off);
                }
            }
        }
    });

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
        let _ = slint::quit_event_loop();
    });

    app.on_theme_selected({
        let weak = app.as_weak();
        move |index| {
            if let Some(app) = weak.upgrade() {
                // 0 = sistem (deneme: koyu), 1 = koyu, 2 = acik
                app.global::<Theme>().set_dark(index != 2);
            }
        }
    });

    // Olcum ve ekran goruntusu icin ortam degiskenleri (yalnizca deneme asamasi).
    if std::env::var_os("GDPI_SETTINGS").is_some() {
        app.set_settings_open(true);
    }
    if std::env::var_os("GDPI_LIGHT").is_some() {
        app.global::<Theme>().set_dark(false);
    }

    app.show()?;
    style_window(&app);

    // GDPI_DEMO=1 iken guc durumu her 3 sn'de degisir (animasyon maliyetini olcmek icin).
    let _demo = std::env::var_os("GDPI_DEMO").map(|_| {
        let timer = Timer::default();
        let weak = app.as_weak();
        timer.start(TimerMode::Repeated, Duration::from_secs(3), move || {
            if let Some(app) = weak.upgrade() {
                app.invoke_power_clicked();
            }
        });
        timer
    });

    slint::run_event_loop()?;
    Ok(())
}

/// Win11'de cercevesiz pencereye yuvarlak kose ve DWM golgesi verir.
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
    });
}
