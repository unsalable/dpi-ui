//! Tepsi menusu: uygulamaya ozel acilir pencere (saf Win32, Slint yuklenmez).
//! Ustte uygulama simgesi ve durum, altinda baglanti anahtari, "Ac" ve "Cik".
//! Sekiller kenar yumusatmali olarak dogrudan piksele cizilir, yazi GDI ClearType ile;
//! koseleri DWM yuvarlar. Menu acikken durum degisirse aninda yenilenir.

use std::cell::RefCell;

use shared::ipc::EngineState;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, EndPaint, FW_NORMAL, FW_SEMIBOLD, GdiFlush, HBITMAP, HDC, HFONT,
    InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CalculatePopupWindowPosition, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetForegroundWindow, HWND_TOPMOST, KillTimer,
    PostMessageW, RegisterClassW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetForegroundWindow, SetTimer, SetWindowPos, TPM_BOTTOMALIGN,
    TPM_RIGHTALIGN, TPM_WORKAREA, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::iconart::{self, Mark};
use crate::winsys::wide;

/// Menunun gosterdigi durum (tepsi surecindeki son bilinen motor durumu).
#[derive(Clone, Default)]
pub struct View {
    pub connected: bool,
    pub state: EngineState,
    pub method: String,
    pub dns: String,
    pub message: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Toggle,
    StartService,
    Open,
    Quit,
}

// ---------------------------------------------------------------- yerlesim (mantiksal px)

const W: f32 = 264.0;
const PAD: f32 = 6.0;
const HEADER_H: f32 = 58.0;
const ROW_H: f32 = 36.0;
const SEP_GAP: f32 = 9.0;
const ROW0: f32 = PAD + HEADER_H + SEP_GAP;
const ROW1: f32 = ROW0 + ROW_H + SEP_GAP;
const ROW2: f32 = ROW1 + ROW_H;
const H: f32 = ROW2 + ROW_H + PAD;
const ROWS: [f32; 3] = [ROW0, ROW1, ROW2];

const WM_CLOSE_MENU: u32 = WM_APP + 20;
const TIMER_KNOB: usize = 1;
/// Menu disina tiklama denetimi (menu one alinamadiginda da kapanabilsin diye).
const TIMER_WATCH: usize = 2;

type Rgb = (u8, u8, u8);

struct Palette {
    bg: Rgb,
    hover: Rgb,
    separator: Rgb,
    border: Rgb,
    label: Rgb,
    secondary: Rgb,
    green: Rgb,
    red: Rgb,
    blue: Rgb,
    switch_off: Rgb,
}

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: (0x2C, 0x2C, 0x2E),
            hover: (0x3A, 0x3A, 0x3C),
            separator: (0x3A, 0x3A, 0x3C),
            border: (0x48, 0x48, 0x4A),
            label: (0xFF, 0xFF, 0xFF),
            secondary: (0x98, 0x98, 0x9F),
            green: (0x30, 0xD1, 0x58),
            red: (0xFF, 0x45, 0x3A),
            blue: (0x0A, 0x84, 0xFF),
            switch_off: (0x48, 0x48, 0x4A),
        }
    } else {
        Palette {
            bg: (0xFF, 0xFF, 0xFF),
            hover: (0xE9, 0xE9, 0xEB),
            separator: (0xE5, 0xE5, 0xEA),
            border: (0xD1, 0xD1, 0xD6),
            label: (0x00, 0x00, 0x00),
            secondary: (0x6E, 0x6E, 0x73),
            green: (0x34, 0xC7, 0x59),
            red: (0xFF, 0x3B, 0x30),
            blue: (0x00, 0x7A, 0xFF),
            switch_off: (0xD8, 0xD8, 0xDD),
        }
    }
}

fn is_on(v: &View) -> bool {
    v.connected && matches!(v.state, EngineState::On | EngineState::Starting)
}

fn status(v: &View, p: &Palette) -> (String, Rgb) {
    if !v.connected {
        return ("Motor hizmeti kapalı".into(), p.secondary);
    }
    match v.state {
        EngineState::On => (format!("Korumalı · {}", v.method), p.green),
        EngineState::Starting => ("Bağlanıyor…".into(), p.blue),
        EngineState::Paused => ("Duraklatıldı".into(), p.secondary),
        EngineState::Error => ("Bağlanamadı".into(), p.red),
        EngineState::Off => ("Kapalı".into(), p.secondary),
    }
}

fn row_label(i: usize, v: &View) -> &'static str {
    match i {
        0 if !v.connected => "Motor hizmetini başlat",
        0 => "Bağlantı",
        1 => "Uygulamayı aç",
        _ => "Çık",
    }
}

fn row_action(i: usize, v: &View) -> Action {
    match i {
        0 if !v.connected => Action::StartService,
        0 => Action::Toggle,
        1 => Action::Open,
        _ => Action::Quit,
    }
}

/// Mantiksal noktanin uzerindeki satir.
fn hit(x: f32, y: f32) -> Option<usize> {
    if !(PAD..=W - PAD).contains(&x) {
        return None;
    }
    ROWS.iter().position(|&top| y >= top && y < top + ROW_H)
}

// ---------------------------------------------------------------- piksel cizimi

/// 32 bit yukaridan asagi DIB uzerine kenar yumusatmali sekiller.
struct Canvas<'a> {
    px: &'a mut [u8],
    w: usize,
    h: usize,
    s: f32,
}

impl Canvas<'_> {
    fn blend(&mut self, x: usize, y: usize, c: Rgb, a: f32) {
        if a <= 0.0 || x >= self.w || y >= self.h {
            return;
        }
        let i = (y * self.w + x) * 4;
        let a = a.min(1.0);
        for (k, v) in [c.2, c.1, c.0].into_iter().enumerate() {
            self.px[i + k] = (v as f32 * a + self.px[i + k] as f32 * (1.0 - a)).round() as u8;
        }
        self.px[i + 3] = 255;
    }

    fn fill(&mut self, c: Rgb) {
        for p in self.px.chunks_exact_mut(4) {
            p.copy_from_slice(&[c.2, c.1, c.0, 255]);
        }
    }

    /// Yuvarlak koseli dikdortgen (mantiksal koordinatlar); r = yari yukseklik ise hap.
    fn rrect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, c: Rgb) {
        let s = self.s;
        let (x0, y0, x1, y1, r) = (x0 * s, y0 * s, x1 * s, y1 * s, r * s);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        for py in (y0.floor().max(0.0) as usize)..(y1.ceil() as usize).min(self.h) {
            for px in (x0.floor().max(0.0) as usize)..(x1.ceil() as usize).min(self.w) {
                let (qx, qy) = ((px as f32 + 0.5 - cx).abs() - hx + r, (py as f32 + 0.5 - cy).abs() - hy + r);
                let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
                self.blend(px, py, c, 0.5 - d);
            }
        }
    }

    fn hline(&mut self, x0: f32, x1: f32, y: f32, c: Rgb) {
        let py = (y * self.s).round() as usize;
        for px in ((x0 * self.s) as usize)..((x1 * self.s) as usize).min(self.w) {
            self.blend(px, py, c, 1.0);
        }
    }

    /// Duz alfali BGRA goruntuyu (x, y) fiziksel noktasina bindirir.
    fn image(&mut self, x: usize, y: usize, size: usize, bgra: &[u8]) {
        for j in 0..size {
            for i in 0..size {
                let k = (j * size + i) * 4;
                let a = bgra[k + 3] as f32 / 255.0;
                self.blend(x + i, y + j, (bgra[k + 2], bgra[k + 1], bgra[k]), a);
            }
        }
    }
}

struct Fonts {
    title: HFONT,
    body: HFONT,
    small: HFONT,
}

fn make_font(px: f32, weight: i32) -> HFONT {
    let face = wide("Segoe UI Variable Text");
    unsafe {
        CreateFontW(
            -(px.round() as i32),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            0,
            face.as_ptr(),
        )
    }
}

fn fonts(s: f32) -> Fonts {
    Fonts { title: make_font(14.0 * s, FW_SEMIBOLD as i32), body: make_font(13.5 * s, FW_NORMAL as i32), small: make_font(12.0 * s, FW_NORMAL as i32) }
}

fn colorref(c: Rgb) -> u32 {
    c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16
}

fn text(dc: HDC, font: HFONT, s: f32, x: f32, y: f32, w: f32, h: f32, t: &str, c: Rgb) {
    let mut rc = RECT { left: (x * s) as i32, top: (y * s) as i32, right: ((x + w) * s) as i32, bottom: ((y + h) * s) as i32 };
    let t: Vec<u16> = t.encode_utf16().collect();
    unsafe {
        SelectObject(dc, font);
        SetTextColor(dc, colorref(c));
        DrawTextW(dc, t.as_ptr(), t.len() as i32, &mut rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
    }
}

/// Menunun bellekteki goruntusu (DIB + DC). Her durum degisikliginde yeniden cizilir.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    bits: *mut u8,
    w: usize,
    h: usize,
    fonts: Fonts,
}

impl Surface {
    fn new(s: f32) -> Option<Surface> {
        let (w, h) = ((W * s).round() as usize, (H * s).round() as usize);
        unsafe {
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if dc.is_null() || bitmap.is_null() || bits.is_null() {
                return None;
            }
            SelectObject(dc, bitmap);
            SetBkMode(dc, TRANSPARENT as i32);
            Some(Surface { dc, bitmap, bits: bits as *mut u8, w, h, fonts: fonts(s) })
        }
    }

    fn pixels(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.w * self.h * 4) }
    }

    /// `knob`: anahtar topunun konumu (0 = kapali, 1 = acik; animasyonda arada).
    fn render(&mut self, v: &View, hover: Option<usize>, knob: f32, dark: bool, s: f32) {
        let p = palette(dark);
        let (w, h) = (self.w, self.h);
        unsafe { GdiFlush() };
        let icon_px = (32.0 * s).round() as usize;
        let icon = iconart::render_bgra(icon_px as u32, Mark::App);
        {
            let mut c = Canvas { px: self.pixels(), w, h, s };
            c.fill(p.bg);
            // Kenar cizgisi (DWM kenarligi olmayan sistemlerde de belli olsun).
            c.rrect(0.0, 0.0, W, H, 8.0, p.border);
            c.rrect(1.0, 1.0, W - 1.0, H - 1.0, 7.0, p.bg);
            if let Some(i) = hover {
                c.rrect(PAD, ROWS[i], W - PAD, ROWS[i] + ROW_H, 6.0, p.hover);
            }
            c.hline(PAD + 8.0, W - PAD - 8.0, PAD + HEADER_H + 4.0, p.separator);
            c.hline(PAD + 8.0, W - PAD - 8.0, ROW0 + ROW_H + 4.0, p.separator);
            c.image((14.0 * s) as usize, ((PAD + 13.0) * s) as usize, icon_px, &icon);

            if v.connected {
                // iOS anahtari: 38x22 hap, 18 px top.
                let (sx, sy) = (W - PAD - 12.0 - 38.0, ROW0 + (ROW_H - 22.0) / 2.0);
                let off = p.switch_off;
                let on = p.green;
                let t = knob.clamp(0.0, 1.0);
                let track = (
                    (off.0 as f32 + (on.0 as f32 - off.0 as f32) * t) as u8,
                    (off.1 as f32 + (on.1 as f32 - off.1 as f32) * t) as u8,
                    (off.2 as f32 + (on.2 as f32 - off.2 as f32) * t) as u8,
                );
                c.rrect(sx, sy, sx + 38.0, sy + 22.0, 11.0, track);
                let kx = sx + 2.0 + 16.0 * t;
                if !dark {
                    c.rrect(kx - 0.5, sy + 1.5, kx + 18.5, sy + 20.5, 9.5, (0xD8, 0xD8, 0xDC));
                }
                c.rrect(kx, sy + 2.0, kx + 18.0, sy + 20.0, 9.0, (0xFF, 0xFF, 0xFF));
            }
        }

        let (title_c, (st, st_c)) = (p.label, status(v, &p));
        let f = &self.fonts;
        text(self.dc, f.title, s, 56.0, PAD + 9.0, W - 70.0, 20.0, "DPI UI", title_c);
        text(self.dc, f.small, s, 56.0, PAD + 29.0, W - 70.0, 18.0, &st, st_c);
        for i in 0..3 {
            let color = if i == 0 && !v.connected { p.blue } else { p.label };
            text(self.dc, f.body, s, PAD + 12.0, ROWS[i], W - 100.0, ROW_H, row_label(i, v), color);
        }
        unsafe { GdiFlush() };
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.dc);
            DeleteObject(self.bitmap);
            DeleteObject(self.fonts.title);
            DeleteObject(self.fonts.body);
            DeleteObject(self.fonts.small);
        }
    }
}

// ---------------------------------------------------------------- pencere

struct Menu {
    hwnd: HWND,
    surface: Surface,
    scale: f32,
    dark: bool,
    view: View,
    hover: Option<usize>,
    knob: f32,
    knob_target: f32,
    tracking: bool,
    /// Menu gercekten on plana gecti mi? Gecmediyse (one alma izni yoksa) Windows'un hemen
    /// geri aldigi etkinlik menuyu kapatmamali; o durumda disariya tiklama denetimi kapatir.
    was_active: bool,
    on_action: fn(Action),
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

impl Menu {
    fn redraw(&mut self) {
        self.surface.render(&self.view, self.hover, self.knob, self.dark, self.scale);
        unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
    }

    fn set_hover(&mut self, h: Option<usize>) {
        if h != self.hover {
            self.hover = h;
            self.redraw();
        }
    }

    fn animate_knob(&mut self) {
        self.knob_target = if is_on(&self.view) { 1.0 } else { 0.0 };
        if (self.knob - self.knob_target).abs() > 0.001 {
            unsafe { SetTimer(self.hwnd, TIMER_KNOB, 15, None) };
        }
    }
}

pub fn is_open() -> bool {
    MENU.with(|m| m.borrow().is_some())
}

/// Acik menuyu yeni durumla yeniler (anahtar yumusakca kayar).
pub fn update(view: &View) {
    MENU.with(|m| {
        if let Some(menu) = m.borrow_mut().as_mut() {
            menu.view = view.clone();
            menu.animate_knob();
            menu.redraw();
        }
    });
}

pub fn close() {
    let hwnd = MENU.with(|m| m.borrow().as_ref().map(|menu| menu.hwnd));
    if let Some(hwnd) = hwnd {
        unsafe { DestroyWindow(hwnd) };
    }
}

fn act(i: usize) {
    let Some((action, cb)) = MENU.with(|m| m.borrow().as_ref().map(|menu| (row_action(i, &menu.view), menu.on_action))) else {
        return;
    };
    match action {
        // Anahtar menuyu kapatmaz: topun kaydigi gorulur, durum gelince kesinlesir.
        Action::Toggle => {
            MENU.with(|m| {
                if let Some(menu) = m.borrow_mut().as_mut() {
                    menu.knob_target = if menu.knob_target > 0.5 { 0.0 } else { 1.0 };
                    unsafe { SetTimer(menu.hwnd, TIMER_KNOB, 15, None) };
                }
            });
            cb(action);
        }
        _ => {
            close();
            cb(action);
        }
    }
}

/// Imlecin yaninda (gorev cubugunun ustunde) menuyu acar.
pub fn show(view: &View, dark: bool, on_action: fn(Action)) {
    close();
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    show_at(pt, view, dark, on_action);
}

pub fn show_at(pt: POINT, view: &View, dark: bool, on_action: fn(Action)) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("GoodbyeDPI3TrayMenu");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("DPI UI").as_ptr(),
            WS_POPUP,
            pt.x,
            pt.y,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let Some(surface) = Surface::new(scale) else {
            DestroyWindow(hwnd);
            return;
        };
        let size = SIZE { cx: surface.w as i32, cy: surface.h as i32 };
        let mut rc = RECT { left: pt.x - size.cx, top: pt.y - size.cy, right: pt.x, bottom: pt.y };
        CalculatePopupWindowPosition(&pt, &size, TPM_RIGHTALIGN | TPM_BOTTOMALIGN | TPM_WORKAREA, std::ptr::null(), &mut rc);

        let corner = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &corner as *const _ as *const _, 4);
        let dark_mode: i32 = dark as i32;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark_mode as *const _ as *const _, 4);
        let border = colorref(palette(dark).border);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, &border as *const _ as *const _, 4);

        let knob = if is_on(view) { 1.0 } else { 0.0 };
        let mut menu = Menu {
            hwnd,
            surface,
            scale,
            dark,
            view: view.clone(),
            hover: None,
            knob,
            knob_target: knob,
            tracking: false,
            was_active: false,
            on_action,
        };
        menu.surface.render(&menu.view, None, knob, dark, scale);
        MENU.with(|m| *m.borrow_mut() = Some(menu));
        // Etkinlestirmeden goster; etkinlik ayrica istenir. One alma izni yoksa menu acik kalir
        // ve disariya tiklaninca (TIMER_WATCH) kapanir.
        SetWindowPos(hwnd, HWND_TOPMOST, rc.left, rc.top, size.cx, size.cy, SWP_SHOWWINDOW | SWP_NOACTIVATE);
        // Disariya tiklaninca kapanmasi icin menu one alinir (tepsi sureci bu izne sahiptir).
        SetForegroundWindow(hwnd);
        let foreground = GetForegroundWindow() == hwnd;
        MENU.with(|m| {
            if let Some(menu) = m.borrow_mut().as_mut() {
                menu.was_active = foreground;
            }
        });
        SetTimer(hwnd, TIMER_WATCH, 50, None);
    }
}

fn logical(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam & 0xFFFF) as i16 as f32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32;
    let s = MENU.with(|m| m.borrow().as_ref().map_or(1.0, |menu| menu.scale));
    (x / s, y / s)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            MENU.with(|m| {
                if let Some(menu) = m.borrow().as_ref() {
                    let s = &menu.surface;
                    unsafe { BitBlt(dc, 0, 0, s.w as i32, s.h as i32, s.dc, 0, 0, SRCCOPY) };
                }
            });
            unsafe { EndPaint(hwnd, &ps) };
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let (x, y) = logical(lparam);
            MENU.with(|m| {
                if let Some(menu) = m.borrow_mut().as_mut() {
                    if !menu.tracking {
                        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        unsafe { TrackMouseEvent(&mut t) };
                        menu.tracking = true;
                    }
                    menu.set_hover(hit(x, y));
                }
            });
            0
        }
        WM_MOUSELEAVE => {
            MENU.with(|m| {
                if let Some(menu) = m.borrow_mut().as_mut() {
                    menu.tracking = false;
                    menu.set_hover(None);
                }
            });
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = logical(lparam);
            if let Some(i) = hit(x, y) {
                act(i);
            }
            0
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            if key == VK_ESCAPE {
                close();
            } else if key == VK_UP || key == VK_DOWN {
                MENU.with(|m| {
                    if let Some(menu) = m.borrow_mut().as_mut() {
                        let next = match (menu.hover, key == VK_DOWN) {
                            (None, true) => 0,
                            (None, false) => 2,
                            (Some(i), true) => (i + 1) % 3,
                            (Some(i), false) => (i + 2) % 3,
                        };
                        menu.set_hover(Some(next));
                    }
                });
            } else if key == VK_RETURN || key == VK_SPACE {
                if let Some(i) = MENU.with(|m| m.borrow().as_ref().and_then(|menu| menu.hover)) {
                    act(i);
                }
            }
            0
        }
        WM_TIMER if wparam == TIMER_KNOB => {
            MENU.with(|m| {
                if let Some(menu) = m.borrow_mut().as_mut() {
                    let d = menu.knob_target - menu.knob;
                    menu.knob = if d.abs() < 0.02 { menu.knob_target } else { menu.knob + d * 0.35 };
                    if menu.knob == menu.knob_target {
                        unsafe { KillTimer(hwnd, TIMER_KNOB) };
                    }
                    menu.redraw();
                }
            });
            0
        }
        WM_TIMER if wparam == TIMER_WATCH => {
            // Menu disinda bir fare dugmesine basildiysa kapan.
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON};
            use windows_sys::Win32::Graphics::Gdi::PtInRect;
            use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
            // On plandaydi ve artik degil (Alt+Tab, baska pencere): kapan.
            let foreground = unsafe { GetForegroundWindow() } == hwnd;
            let lost = MENU.with(|m| {
                let mut m = m.borrow_mut();
                let Some(menu) = m.as_mut() else { return false };
                let lost = menu.was_active && !foreground;
                menu.was_active |= foreground;
                lost
            });
            let down = [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|&k| unsafe { GetAsyncKeyState(k as i32) } as u16 & 0x8000 != 0);
            if lost {
                unsafe { PostMessageW(hwnd, WM_CLOSE_MENU, 0, 0) };
            } else if down {
                let mut pt = POINT { x: 0, y: 0 };
                let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                unsafe {
                    GetCursorPos(&mut pt);
                    GetWindowRect(hwnd, &mut rc);
                    if PtInRect(&rc, pt) == 0 {
                        PostMessageW(hwnd, WM_CLOSE_MENU, 0, 0);
                    }
                }
            }
            0
        }
        WM_ACTIVATE => {
            let active = (wparam & 0xFFFF) as u32 != WA_INACTIVE;
            let close = !active && MENU.with(|m| m.borrow().as_ref().is_some_and(|menu| menu.was_active));
            if close {
                // Baska bir yere gecildi: menu kapanir (mesaj isleyicisinin disinda).
                unsafe { PostMessageW(hwnd, WM_CLOSE_MENU, 0, 0) };
            }
            0
        }
        WM_CLOSE_MENU => {
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            unsafe {
                KillTimer(hwnd, TIMER_KNOB);
                KillTimer(hwnd, TIMER_WATCH);
            }
            // Surface burada birakilir (DC, bitmap, yazi tipleri).
            let menu = MENU.with(|m| m.borrow_mut().take());
            drop(menu);
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Testler: menunun cizimini (koyu/acik, durumlar, uzerine gelme) BGRA olarak dondurur.
#[cfg(test)]
pub fn render_for_test(view: &View, hover: Option<usize>, dark: bool, scale: f32) -> (usize, usize, Vec<u8>) {
    let mut s = Surface::new(scale).expect("yuzey");
    s.render(view, hover, if is_on(view) { 1.0 } else { 0.0 }, dark, scale);
    let (w, h) = (s.w, s.h);
    (w, h, s.pixels().to_vec())
}

#[cfg(test)]
pub fn hit_for_test(x: f32, y: f32) -> Option<usize> {
    hit(x, y)
}

#[cfg(test)]
pub fn action_for_test(i: usize, v: &View) -> Action {
    row_action(i, v)
}
