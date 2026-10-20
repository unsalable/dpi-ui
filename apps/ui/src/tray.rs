//! Tepsi sureci (`GoodbyeDPI3.exe --tray`): saf Win32, Slint yuklenmez (~2 MB).
//! Simge motor durumunu gosterir; sol tik pencereyi acar, sag tik uygulamaya ozel menu
//! (`traymenu`).
//! Explorer yeniden baslarsa (TaskbarCreated) simge yeniden eklenir.

use std::sync::{Mutex, OnceLock};

use shared::ipc::{EngineState, Event, Request};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, GetSystemMetrics, HICON, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_DESTROY,
    WM_LBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

use crate::client::{Client, Incoming};
use crate::iconart::Mark;
use crate::traymenu::{self, Action, View};
use crate::winsys::{self, wide};

const WM_TRAY: u32 = WM_APP + 1;
const WM_STATE: u32 = WM_APP + 2;
/// Baska bir surec "cik" dedi (pencere kapatildi ya da `--quit`).
const WM_QUIT_ALL: u32 = WM_APP + 3;
const NIN_SELECT: u32 = 0x0400;
const NIN_KEYSELECT: u32 = 0x0401;

struct Tray {
    hwnd: usize,
    icons: [usize; 4], // on, off, busy, error
    view: View,
    client: Option<Client>,
    taskbar_created: u32,
}

static TRAY: OnceLock<Mutex<Tray>> = OnceLock::new();

fn tray() -> std::sync::MutexGuard<'static, Tray> {
    TRAY.get().expect("tepsi kurulmadi").lock().unwrap()
}

fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

fn tooltip(v: &View) -> String {
    let state = if !v.connected {
        "Motor hizmeti kapalı".to_owned()
    } else {
        match v.state {
            EngineState::On => format!("Korumalı · {} · {}", v.method, v.dns),
            EngineState::Starting => "Bağlanıyor…".to_owned(),
            EngineState::Paused => "Duraklatıldı".to_owned(),
            EngineState::Error => format!("Bağlanamadı: {}", v.message.clone().unwrap_or_default()),
            EngineState::Off => "Kapalı".to_owned(),
        }
    };
    let t = format!("GoodbyeDPI — {state}");
    t.chars().take(120).collect()
}

fn icon_for(t: &Tray) -> HICON {
    let i = match (t.view.connected, t.view.state) {
        (false, _) => 1,
        (true, EngineState::On) => 0,
        (true, EngineState::Starting) => 2,
        (true, EngineState::Error) => 3,
        _ => 1,
    };
    t.icons[i] as HICON
}

fn update_icon(add: bool) {
    let t = tray();
    let hwnd = t.hwnd as HWND;
    let mut nid = notify_data(hwnd);
    nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon_for(&t);
    let tip: Vec<u16> = tooltip(&t.view).encode_utf16().collect();
    let n = tip.len().min(nid.szTip.len() - 1);
    nid.szTip[..n].copy_from_slice(&tip[..n]);
    drop(t);
    unsafe {
        if add {
            Shell_NotifyIconW(NIM_ADD, &nid);
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &nid);
        } else {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }
}

fn open_window() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
}

/// Menunun temasi: uygulamada secilen tema; "sistem" ise gorev cubugunun temasi.
fn menu_is_dark() -> bool {
    match crate::prefs::load().theme {
        1 => true,
        2 => false,
        _ => !winsys::system_uses_light_taskbar(),
    }
}

fn show_menu() {
    let v = tray().view.clone();
    traymenu::show(&v, menu_is_dark(), on_menu_action);
}

fn on_menu_action(action: Action) {
    match action {
        Action::Toggle => {
            let (client, on) = {
                let t = tray();
                (t.client.clone(), matches!(t.view.state, EngineState::On | EngineState::Starting))
            };
            if let Some(c) = client {
                c.send(if on { Request::Disconnect } else { Request::Connect });
            }
        }
        Action::StartService => {
            let _ = winsys::start_service();
        }
        Action::Open => open_window(),
        Action::Quit => {
            // Acik pencere de kapanir; motor hizmeti kendi ayarina gore calismaya devam eder.
            winsys::signal_quit();
            let hwnd = tray().hwnd as HWND;
            unsafe { DestroyWindow(hwnd) };
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = (lparam as u32) & 0xFFFF;
            match event {
                WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT => {
                    traymenu::close();
                    open_window();
                }
                WM_CONTEXTMENU => show_menu(),
                _ => {}
            }
            0
        }
        WM_STATE => {
            update_icon(false);
            if traymenu::is_open() {
                let v = tray().view.clone();
                traymenu::update(&v);
            }
            0
        }
        WM_QUIT_ALL => {
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            let nid = notify_data(hwnd);
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &nid);
                PostQuitMessage(0);
            }
            0
        }
        m if TRAY.get().is_some() && m == tray().taskbar_created => {
            update_icon(true);
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub fn run() -> i32 {
    if !winsys::claim_mutex(&winsys::instance_name("Local\\GoodbyeDPI3.Tray")) {
        return 0;
    }
    let quit = winsys::quit_event() as usize;
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("GoodbyeDPI3Tray");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        // Gizli ust duzey pencere: TaskbarCreated yayinini yalnizca bunlar alir.
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("GoodbyeDPI").as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
        if hwnd.is_null() {
            return 1;
        }
        let size = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
        let icons = [
            winsys::make_icon(size, Mark::On) as usize,
            winsys::make_icon(size, Mark::Off) as usize,
            winsys::make_icon(size, Mark::Busy) as usize,
            winsys::make_icon(size, Mark::Error) as usize,
        ];
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let _ = TRAY.set(Mutex::new(Tray { hwnd: hwnd as usize, icons, view: View::default(), client: None, taskbar_created }));
        update_icon(true);

        let hwnd_val = hwnd as usize;
        let client = Client::start("tepsi", move |inc| {
            {
                let mut t = tray();
                match inc {
                    Incoming::Connected(state) => {
                        t.view = View {
                            connected: true,
                            state: state.engine.state,
                            method: state.engine.method_name,
                            dns: state.engine.dns_name,
                            message: state.engine.message,
                        };
                    }
                    Incoming::Disconnected => t.view.connected = false,
                    Incoming::Event(Event::Engine { status }) => {
                        t.view.state = status.state;
                        t.view.method = status.method_name;
                        t.view.dns = status.dns_name;
                        t.view.message = status.message;
                    }
                    Incoming::Event(_) => return,
                }
            }
            PostMessageW(hwnd_val as HWND, WM_STATE, 0, 0);
        });
        tray().client = Some(client);

        // Pencere "her seyi kapat" dediginde ya da `--quit` ile tepsi de kapanir.
        std::thread::Builder::new()
            .name("cikis".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
                use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};
                if WaitForSingleObject(quit as _, INFINITE) == WAIT_OBJECT_0 {
                    PostMessageW(hwnd_val as HWND, WM_QUIT_ALL, 0, 0);
                }
            })
            .ok();

        // Testler: menu acilista acilir (ekran goruntusu ve tiklama denemeleri icin).
        if std::env::var_os("GDPI3_TEST").is_some() && std::env::var_os("GDPI3_TRAY_MENU").is_some() {
            PostMessageW(hwnd, WM_TRAY, 0, WM_CONTEXTMENU as isize);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    0
}
