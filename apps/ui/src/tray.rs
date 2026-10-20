//! Tepsi sureci (`GoodbyeDPI3.exe --tray`): saf Win32, Slint yuklenmez (~2 MB).
//! Simge motor durumunu gosterir; sol tik pencereyi acar, sag tik kisa menu.
//! Explorer yeniden baslarsa (TaskbarCreated) simge yeniden eklenir.

use std::sync::{Mutex, OnceLock};

use shared::ipc::{EngineState, Event, Request};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetMessageW, GetSystemMetrics, HICON, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, SetMenuDefaultItem, TPM_BOTTOMALIGN,
    TPM_RIGHTBUTTON, TPM_RETURNCMD, TrackPopupMenu, TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP,
    WNDCLASSW, WS_OVERLAPPED,
};

use crate::client::{Client, Incoming};
use crate::iconart::Mark;
use crate::winsys::{self, wide};

const WM_TRAY: u32 = WM_APP + 1;
const WM_STATE: u32 = WM_APP + 2;
const NIN_SELECT: u32 = 0x0400;
const NIN_KEYSELECT: u32 = 0x0401;
const CMD_TOGGLE: usize = 1;
const CMD_OPEN: usize = 2;
const CMD_EXIT: usize = 3;

#[derive(Clone, Default)]
struct View {
    connected: bool,
    state: EngineState,
    method: String,
    dns: String,
    message: Option<String>,
}

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

/// Sag tik menusunun koyu temaya uymasi (uxtheme sirasiz islevleri; yoksa sessizce gecer).
fn allow_dark_menus() {
    unsafe {
        let lib = LoadLibraryW(wide("uxtheme.dll").as_ptr());
        if lib.is_null() {
            return;
        }
        if let Some(f) = GetProcAddress(lib, 135 as *const u8) {
            let set_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set_mode(1); // AllowDark: sistem temasini izle
        }
        if let Some(f) = GetProcAddress(lib, 136 as *const u8) {
            let flush: extern "system" fn() = std::mem::transmute(f);
            flush();
        }
    }
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

fn show_menu(hwnd: HWND) {
    let v = tray().view.clone();
    unsafe {
        let menu = CreatePopupMenu();
        let (label, flags) = if !v.connected {
            ("Motor hizmeti kapalı", MF_STRING | MF_GRAYED)
        } else if matches!(v.state, EngineState::On | EngineState::Starting) {
            ("Bağlantıyı kes", MF_STRING)
        } else {
            ("Bağlan", MF_STRING)
        };
        AppendMenuW(menu, flags, CMD_TOGGLE, wide(label).as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_OPEN, wide("GoodbyeDPI'ı aç").as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, CMD_EXIT, wide("Tepsiden çık").as_ptr());
        SetMenuDefaultItem(menu, CMD_OPEN as u32, 0);

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Menu disina tiklaninca kapanmasi icin pencere one alinmali (Win32 kurali).
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, std::ptr::null()) as usize;
        DestroyMenu(menu);

        match cmd {
            CMD_TOGGLE => {
                let client = tray().client.clone();
                if let Some(c) = client {
                    let req = if matches!(v.state, EngineState::On | EngineState::Starting) { Request::Disconnect } else { Request::Connect };
                    c.send(req);
                }
            }
            CMD_OPEN => open_window(),
            CMD_EXIT => {
                DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = (lparam as u32) & 0xFFFF;
            match event {
                WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT => open_window(),
                WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_STATE => {
            update_icon(false);
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
    if !winsys::claim_mutex("Local\\GoodbyeDPI3.Tray") {
        return 0;
    }
    allow_dark_menus();
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

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    0
}
