//! Tepsi menusu: uygulamaya ozel acilir pencere (saf Win32, Slint yuklenmez).
//! Uygulamanin ana ekraninin kucugu: solda baglanti simgesi (uygulamadakiyle ayni cizim ve
//! renkler), yaninda durum ve anahtar; altta gruplu satirlar (Uygulamayi ac, Cik).
//! Sekiller kenar yumusatmali olarak dogrudan piksele cizilir (uzaklik alanlari), yazi GDI
//! ClearType ile; koseleri DWM yuvarlar. Durum degisince simge gecis yapar, baglanirken
//! yaylar doner, anahtar kayar, uzerine gelinen satir yavasca belirir.

use std::cell::RefCell;
use std::time::Instant;

use shared::ipc::EngineState;
use shared::t;
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
    CalculatePopupWindowPosition, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetForegroundWindow, HWND_TOPMOST, IDC_ARROW,
    KillTimer, LoadCursorW, PostMessageW, RegisterClassW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetForegroundWindow, SetTimer, SetWindowPos, TPM_BOTTOMALIGN,
    TPM_RIGHTALIGN, TPM_WORKAREA, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

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

const W: f32 = 288.0;
const PAD: f32 = 8.0;
/// Ust bolum: simge, durum ve anahtar (tamami tiklanir).
const HEAD_H: f32 = 68.0;
const GLYPH: f32 = 40.0;
const ROW_H: f32 = 44.0;
const GROUP_TOP: f32 = PAD + HEAD_H + 6.0;
const ROW1: f32 = GROUP_TOP;
const ROW2: f32 = GROUP_TOP + ROW_H;
const H: f32 = ROW2 + ROW_H + PAD;
/// Satir karosu ve metin hizasi (uygulamadaki ayar satirlari gibi).
const TILE: f32 = 28.0;
const TEXT_X: f32 = PAD + 12.0 + TILE + 12.0;
/// Anahtar: uygulamadakiyle ayni oranlar (46x28 yerine 40x24).
const SW_W: f32 = 40.0;
const SW_H: f32 = 24.0;

const WM_CLOSE_MENU: u32 = WM_APP + 20;
/// Butun hareketler (anahtar, satir vurgusu, simge) tek zamanlayiciyla ilerler.
const TIMER_ANIM: usize = 1;
/// Menu disina tiklama denetimi (menu one alinamadiginda da kapanabilsin diye).
const TIMER_WATCH: usize = 2;
/// Simge gecisinin suresi (ms).
const LOOK_MS: f32 = 460.0;
/// Baglanirken donus hizi (derece / ms; uygulamadaki gibi 1,2 sn'de bir tur).
const SPIN: f32 = 0.3;

type Rgb = (u8, u8, u8);

/// Uygulamanin renkleri (tokens.slint).
struct Palette {
    bg: Rgb,
    cell: Rgb,
    cell_hover: Rgb,
    head_hover: Rgb,
    separator: Rgb,
    border: Rgb,
    label: Rgb,
    secondary: Rgb,
    green: Rgb,
    red: Rgb,
    blue: Rgb,
    gray: Rgb,
    switch_off: Rgb,
}

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: (0x00, 0x00, 0x00),
            cell: (0x1C, 0x1C, 0x1E),
            cell_hover: (0x2C, 0x2C, 0x2E),
            head_hover: (0x1C, 0x1C, 0x1E),
            separator: (0x38, 0x38, 0x3A),
            border: (0x38, 0x38, 0x3A),
            label: (0xFF, 0xFF, 0xFF),
            secondary: (0x8D, 0x8D, 0x93),
            green: (0x30, 0xD1, 0x58),
            red: (0xFF, 0x45, 0x3A),
            blue: (0x0A, 0x84, 0xFF),
            gray: (0x63, 0x63, 0x66),
            switch_off: (0x39, 0x39, 0x3D),
        }
    } else {
        Palette {
            bg: (0xF2, 0xF2, 0xF7),
            cell: (0xFF, 0xFF, 0xFF),
            cell_hover: (0xE5, 0xE5, 0xEA),
            head_hover: (0xE5, 0xE5, 0xEA),
            separator: (0xD1, 0xD1, 0xD6),
            border: (0xD1, 0xD1, 0xD6),
            label: (0x00, 0x00, 0x00),
            secondary: (0x6E, 0x6E, 0x73),
            green: (0x34, 0xC7, 0x59),
            red: (0xFF, 0x3B, 0x30),
            blue: (0x00, 0x7A, 0xFF),
            gray: (0x8E, 0x8E, 0x93),
            // Acik zeminde (F2F2F7) uygulamadaki E9E9EB secilmiyor; bir ton koyusu.
            switch_off: (0xD8, 0xD8, 0xDD),
        }
    }
}

fn is_on(v: &View) -> bool {
    v.connected && matches!(v.state, EngineState::On | EngineState::Starting)
}

/// Simgenin gorunumu (uygulamadaki ConnectGlyph durumlari).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Look {
    /// Gri parantezler ve dunya: kapali, duraklatildi, hizmet yok.
    Idle,
    /// Kirmizi parantezler ve dunya.
    Error,
    /// Cemberde donen dort yay.
    Busy,
    /// Yesil halka ve onay isareti.
    On,
}

fn look(v: &View) -> Look {
    if !v.connected {
        return Look::Idle;
    }
    match v.state {
        EngineState::On => Look::On,
        EngineState::Starting => Look::Busy,
        EngineState::Error => Look::Error,
        EngineState::Off | EngineState::Paused => Look::Idle,
    }
}

/// Baslik ve alt satir: uygulamanin ana ekranindaki metinlerle ayni.
fn status(v: &View) -> (String, String) {
    if !v.connected {
        return (t!("Motor hizmeti kapalı", "Engine service is off"), t!("Başlatmak için dokun", "Tap to start"));
    }
    match v.state {
        EngineState::On => {
            let parts: Vec<&str> = [v.method.as_str(), v.dns.as_str()].into_iter().filter(|p| !p.is_empty()).collect();
            (t!("Korumalı", "Protected"), parts.join(" · "))
        }
        EngineState::Starting => (t!("Bağlanıyor…", "Connecting…"), v.method.clone()),
        EngineState::Paused => (
            t!("Duraklatıldı", "Paused"),
            v.message.clone().unwrap_or_else(|| t!("Bir uygulama için geçici olarak durduruldu", "Temporarily stopped for an app")),
        ),
        EngineState::Error => (
            t!("Bağlanamadı", "Couldn't connect"),
            v.message.clone().unwrap_or_else(|| t!("Yeniden denemek için dokun", "Tap to try again")),
        ),
        EngineState::Off => (t!("Kapalı", "Off"), t!("Bağlanmak için dokun", "Tap to connect")),
    }
}

fn row_label(i: usize) -> String {
    match i {
        1 => t!("Uygulamayı aç", "Open app"),
        _ => t!("Çık", "Quit"),
    }
}

/// 0 = ust bolum (baglan/kes ya da hizmeti baslat), 1 = uygulamayi ac, 2 = cik.
fn row_action(i: usize, v: &View) -> Action {
    match i {
        0 if !v.connected => Action::StartService,
        0 => Action::Toggle,
        1 => Action::Open,
        _ => Action::Quit,
    }
}

/// Mantiksal noktanin uzerindeki bolum.
fn hit(x: f32, y: f32) -> Option<usize> {
    if !(PAD..=W - PAD).contains(&x) {
        return None;
    }
    if (PAD..PAD + HEAD_H).contains(&y) {
        return Some(0);
    }
    [ROW1, ROW2].iter().position(|&top| y >= top && y < top + ROW_H).map(|i| i + 1)
}

// ---------------------------------------------------------------- hareket

/// Cizim anindaki hareket durumu.
#[derive(Clone, Copy)]
struct Anim {
    /// Anahtar topu: 0 = kapali, 1 = acik.
    knob: f32,
    /// Bolumlerin vurgusu (0..1).
    hover: [f32; 3],
    /// Baglanirken yaylarin acisi (derece).
    spin: f32,
    /// Onceki gorunum ve gecisin ilerlemesi (1 = bitti).
    prev: Look,
    t: f32,
}

impl Anim {
    fn settled(v: &View) -> Anim {
        Anim { knob: if is_on(v) { 1.0 } else { 0.0 }, hover: [0.0; 3], spin: 0.0, prev: look(v), t: 1.0 }
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

// ---------------------------------------------------------------- sekiller (birim kutu)

fn seg(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - ax - t * dx).hypot(py - ay - t * dy)
}

/// a0'dan saat yonunde `sweep` derece giden yay (acilar tepeden).
fn arc(px: f32, py: f32, cx: f32, cy: f32, r: f32, a0: f32, sweep: f32) -> f32 {
    let ang = (px - cx).atan2(-(py - cy)).to_degrees().rem_euclid(360.0);
    if (ang - a0).rem_euclid(360.0) <= sweep {
        return ((px - cx).hypot(py - cy) - r).abs();
    }
    let end = |a: f32| {
        let a = a.to_radians();
        (px - cx - r * a.sin()).hypot(py - cy + r * a.cos())
    };
    end(a0).min(end(a0 + sweep))
}

/// Yuvarlak koseli dikdortgenin cevresine uzaklik.
fn round_box(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let (cx, cy, hx, hy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let (qx, qy) = ((px - cx).abs() - hx + r, (py - cy).abs() - hy + r);
    ((qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r).abs()
}

/// Kapali simge (100'luk kutu): dort kose parantezi (kalinlik 6) ve dunya (3,8).
fn brackets_globe(x: f32, y: f32) -> f32 {
    // Parantezler iki eksene gore simetrik: nokta sol ust koseye katlanir.
    let (fx, fy) = (if x > 50.0 { 100.0 - x } else { x }, if y > 50.0 { 100.0 - y } else { y });
    let corner = seg(fx, fy, 19.0, 6.0, 35.0, 6.0).min(arc(fx, fy, 19.0, 19.0, 13.0, 270.0, 90.0)).min(seg(fx, fy, 6.0, 19.0, 6.0, 35.0));
    let (dx, dy) = (x - 50.0, y - 50.0);
    let (a, b) = (17.68f32, 25.0f32);
    let k = ((dx / a).powi(2) + (dy / b).powi(2)).sqrt();
    let grad = ((dx / (a * a)).powi(2) + (dy / (b * b)).powi(2)).sqrt();
    let meridians = if grad > 0.0 { ((k - 1.0) * k / grad).abs() } else { b };
    let globe = (dx.hypot(dy) - 25.0).abs().min(seg(x, y, 25.0, 50.0, 75.0, 50.0)).min(seg(x, y, 50.0, 25.0, 50.0, 75.0)).min(meridians);
    (corner - 3.0).min(globe - 1.9)
}

/// Cemberdeki dort yay; `gap` her ucta acik kalan aci (0 = tam halka), `r` yaricap.
fn arcs(x: f32, y: f32, r: f32, spin: f32, gap: f32) -> f32 {
    if gap <= 0.01 {
        return ((x - 50.0).hypot(y - 50.0) - r).abs() - 3.0;
    }
    (0..4).map(|k| arc(x, y, 50.0, 50.0, r, spin + 90.0 * k as f32 + gap, 90.0 - 2.0 * gap)).fold(f32::MAX, f32::min) - 3.0
}

/// Onay isareti (31,51) -> (44,64) -> (69,37), `drawn` kadari cizili (kalinlik 7).
fn check(x: f32, y: f32, drawn: f32) -> f32 {
    let d = drawn.clamp(0.0, 1.0) * 55.18;
    if d < 0.2 {
        return f32::MAX;
    }
    if d <= 18.38 {
        return seg(x, y, 31.0, 51.0, 31.0 + 13.0 * d / 18.38, 51.0 + 13.0 * d / 18.38) - 3.5;
    }
    let e = (d - 18.38) / 36.8;
    seg(x, y, 31.0, 51.0, 44.0, 64.0).min(seg(x, y, 44.0, 64.0, 44.0 + 25.0 * e, 64.0 - 27.0 * e)) - 3.5
}

/// Satir simgeleri (24'luk kutu, kalinlik 1,9): uygulama penceresi ve guc dugmesi.
fn icon_open(x: f32, y: f32) -> f32 {
    round_box(x, y, 4.5, 5.5, 19.5, 18.5, 3.0).min(seg(x, y, 4.5, 9.5, 19.5, 9.5)) - 0.95
}

fn icon_quit(x: f32, y: f32) -> f32 {
    arc(x, y, 12.0, 13.0, 6.5, 40.0, 280.0).min(seg(x, y, 12.0, 4.5, 12.0, 11.0)) - 1.0
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

    /// Yuvarlak koseli dolu dikdortgen (mantiksal); ust ve alt koselerin yaricapi ayri.
    fn rrect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r_top: f32, r_bottom: f32, c: Rgb, a: f32) {
        if a <= 0.0 {
            return;
        }
        let s = self.s;
        let (x0, y0, x1, y1) = (x0 * s, y0 * s, x1 * s, y1 * s);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hx, hy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
        for py in (y0.floor().max(0.0) as usize)..(y1.ceil() as usize).min(self.h) {
            let r = if (py as f32 + 0.5) < cy { r_top } else { r_bottom } * s;
            for px in (x0.floor().max(0.0) as usize)..(x1.ceil() as usize).min(self.w) {
                let (qx, qy) = ((px as f32 + 0.5 - cx).abs() - hx + r, (py as f32 + 0.5 - cy).abs() - hy + r);
                let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
                self.blend(px, py, c, (0.5 - d).clamp(0.0, 1.0) * a);
            }
        }
    }

    fn hline(&mut self, x0: f32, x1: f32, y: f32, c: Rgb) {
        let py = (y * self.s).round() as usize;
        for px in ((x0 * self.s) as usize)..((x1 * self.s) as usize).min(self.w) {
            self.blend(px, py, c, 1.0);
        }
    }

    /// Cizgi sekli: (x, y) mantiksal noktasinda `size` boyunda, `units` birimlik kutuda;
    /// `sd` noktanin cizginin kenarina birim cinsinden uzakligi (icerde eksi).
    fn shape(&mut self, x: f32, y: f32, size: f32, units: f32, c: Rgb, a: f32, sd: impl Fn(f32, f32) -> f32) {
        if a <= 0.0 {
            return;
        }
        let k = size * self.s / units;
        let (ox, oy) = (x * self.s, y * self.s);
        let n = (size * self.s).ceil() as usize + 2;
        let (bx, by) = (ox.floor().max(1.0) as usize - 1, oy.floor().max(1.0) as usize - 1);
        for py in by..(by + n).min(self.h) {
            for px in bx..(bx + n).min(self.w) {
                let (ux, uy) = ((px as f32 + 0.5 - ox) / k, (py as f32 + 0.5 - oy) / k);
                let d = sd(ux, uy);
                if d < 1.0 {
                    self.blend(px, py, c, (0.5 - d * k).clamp(0.0, 1.0) * a);
                }
            }
        }
    }
}

/// Baglanti simgesi: bir gorunum, gecis ilerlemesiyle (`t`: girisin 0..1'i).
fn draw_look(c: &mut Canvas, p: &Palette, look: Look, prev: Look, t: f32, spin: f32, alpha: f32) {
    let (x, y) = (PAD + 12.0, PAD + (HEAD_H - GLYPH) / 2.0);
    match look {
        Look::Idle | Look::Error => {
            let color = if look == Look::Error { p.red } else { p.secondary };
            // Hafif buyuyerek yerine oturur.
            let grow = 0.9 + 0.1 * ease_out(t);
            let g = GLYPH * grow;
            c.shape(x + (GLYPH - g) / 2.0, y + (GLYPH - g) / 2.0, g, 100.0, color, alpha, brackets_globe);
        }
        Look::Busy => {
            // Yaylar 44'luk cemberden iceri (38) kayarak ayrilir ve doner.
            let e = ease_out(t);
            let r = 44.0 - 6.0 * e;
            let gap = 22.0 * e;
            c.shape(x, y, GLYPH, 100.0, p.label, alpha, move |ux, uy| arcs(ux, uy, r, spin, gap));
        }
        Look::On => {
            // Taramadan geldiyse yaylar boslugu kapatip halkaya kilitlenir; yoksa halka
            // olusur. Onay isareti ardindan cizilir; renk yesile doner.
            let from_busy = prev == Look::Busy;
            let e = ease_out((t / 0.6).min(1.0));
            let (r, gap) = if from_busy { (38.0 + 6.0 * e, 22.0 * (1.0 - e)) } else { (44.0, 0.0) };
            let color = if from_busy { mix(p.label, p.green, e) } else { p.green };
            let drawn = ease_out(((t - 0.35) / 0.65).max(0.0));
            c.shape(x, y, GLYPH, 100.0, color, alpha, move |ux, uy| arcs(ux, uy, r, spin, gap).min(check(ux, uy, drawn)));
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
    Fonts { title: make_font(15.0 * s, FW_SEMIBOLD as i32), body: make_font(14.0 * s, FW_NORMAL as i32), small: make_font(12.0 * s, FW_NORMAL as i32) }
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

    fn render(&mut self, v: &View, a: &Anim, dark: bool, s: f32) {
        let p = palette(dark);
        let (w, h) = (self.w, self.h);
        unsafe { GdiFlush() };
        {
            let mut c = Canvas { px: self.pixels(), w, h, s };
            c.fill(p.bg);
            // Kenar cizgisi (DWM kenarligi olmayan sistemlerde de belli olsun).
            c.rrect(0.0, 0.0, W, H, 8.0, 8.0, p.border, 1.0);
            c.rrect(1.0, 1.0, W - 1.0, H - 1.0, 7.0, 7.0, p.bg, 1.0);

            // Ust bolum: uzerine gelince hafif zemin.
            c.rrect(PAD, PAD, W - PAD, PAD + HEAD_H, 12.0, 12.0, p.head_hover, a.hover[0]);

            // Simge: onceki gorunum soner, yenisi girer. Taramadan baglantiya gecis bir devam:
            // ayni yaylar kapanir, eskisi ayrica cizilmez.
            let now = look(v);
            let continues = a.prev == Look::Busy && now == Look::On;
            if a.t < 1.0 && a.prev != now && !continues {
                let fade = 1.0 - (a.t / 0.4).min(1.0);
                draw_look(&mut c, &p, a.prev, a.prev, 1.0, a.spin, fade);
            }
            let enter = if a.prev != now && !continues { (a.t / 0.3).min(1.0) } else { 1.0 };
            draw_look(&mut c, &p, now, a.prev, a.t, a.spin, enter);

            if v.connected {
                let (sx, sy) = (W - PAD - 12.0 - SW_W, PAD + (HEAD_H - SW_H) / 2.0);
                let k = a.knob.clamp(0.0, 1.0);
                c.rrect(sx, sy, sx + SW_W, sy + SW_H, SW_H / 2.0, SW_H / 2.0, mix(p.switch_off, p.green, k), 1.0);
                let kx = sx + 2.0 + (SW_W - SW_H) * k;
                if !dark {
                    c.rrect(kx - 0.5, sy + 1.5, kx + SW_H - 3.5, sy + SW_H - 1.5, 10.5, 10.5, (0xD8, 0xD8, 0xDC), 1.0);
                }
                c.rrect(kx, sy + 2.0, kx + SW_H - 4.0, sy + SW_H - 2.0, 10.0, 10.0, (0xFF, 0xFF, 0xFF), 1.0);
            }

            // Gruplu satirlar.
            c.rrect(PAD, GROUP_TOP, W - PAD, GROUP_TOP + 2.0 * ROW_H, 10.0, 10.0, p.cell, 1.0);
            c.rrect(PAD, ROW1, W - PAD, ROW1 + ROW_H, 10.0, 0.0, p.cell_hover, a.hover[1]);
            c.rrect(PAD, ROW2, W - PAD, ROW2 + ROW_H, 0.0, 10.0, p.cell_hover, a.hover[2]);
            c.hline(TEXT_X, W - PAD, ROW2 - 0.5, p.separator);
            for (i, top) in [(1usize, ROW1), (2, ROW2)] {
                let (tx, ty) = (PAD + 12.0, top + (ROW_H - TILE) / 2.0);
                let tile = if i == 1 { p.blue } else { p.gray };
                c.rrect(tx, ty, tx + TILE, ty + TILE, 7.0, 7.0, tile, 1.0);
                let icon: fn(f32, f32) -> f32 = if i == 1 { icon_open } else { icon_quit };
                c.shape(tx + 4.0, ty + 4.0, TILE - 8.0, 24.0, (0xFF, 0xFF, 0xFF), 1.0, icon);
            }
        }

        let (title, sub) = status(v);
        let f = &self.fonts;
        let text_x = PAD + 12.0 + GLYPH + 14.0;
        let text_w = if v.connected { W - PAD - 12.0 - SW_W - 10.0 - text_x } else { W - PAD - 12.0 - text_x };
        let sub_color = if v.connected { p.secondary } else { p.blue };
        if sub.is_empty() {
            text(self.dc, f.title, s, text_x, PAD, text_w, HEAD_H, &title, p.label);
        } else {
            text(self.dc, f.title, s, text_x, PAD + 14.0, text_w, 21.0, &title, p.label);
            text(self.dc, f.small, s, text_x, PAD + 35.0, text_w, 18.0, &sub, sub_color);
        }
        for (i, top) in [(1usize, ROW1), (2, ROW2)] {
            text(self.dc, f.body, s, TEXT_X, top, W - PAD - 12.0 - TEXT_X, ROW_H, &row_label(i), p.label);
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
    /// Klavye ya da fareyle secili bolum.
    hover: Option<usize>,
    anim: Anim,
    knob_target: f32,
    last_tick: Instant,
    animating: bool,
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
        self.surface.render(&self.view, &self.anim, self.dark, self.scale);
        unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
    }

    /// Hareket varsa zamanlayiciyi baslatir (60 kare/sn); hepsi durunca kendisi durur.
    fn wake(&mut self) {
        if !self.animating {
            self.animating = true;
            self.last_tick = Instant::now();
            unsafe { SetTimer(self.hwnd, TIMER_ANIM, 15, None) };
        }
    }

    fn set_hover(&mut self, h: Option<usize>) {
        if h != self.hover {
            self.hover = h;
            self.wake();
        }
    }

    /// Bir kare ilerler; hareket kalmadiysa false.
    fn step(&mut self) -> bool {
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32() * 1000.0;
        self.last_tick = now;
        let a = &mut self.anim;
        let mut moving = false;
        // Anahtar ve vurgu: hedefe ustel yaklasma (yaklasik 120 ms'de yerine oturur).
        let k = 1.0 - (-dt / 45.0).exp();
        let d = self.knob_target - a.knob;
        a.knob = if d.abs() < 0.01 { self.knob_target } else { a.knob + d * k };
        moving |= a.knob != self.knob_target;
        for (i, hv) in a.hover.iter_mut().enumerate() {
            let target = if self.hover == Some(i) { 1.0 } else { 0.0 };
            let d = target - *hv;
            *hv = if d.abs() < 0.02 { target } else { *hv + d * (1.0 - (-dt / 40.0).exp()) };
            moving |= *hv != target;
        }
        if a.t < 1.0 {
            a.t = (a.t + dt / LOOK_MS).min(1.0);
            moving = true;
        }
        let now_look = look(&self.view);
        if now_look == Look::Busy || (a.t < 1.0 && (a.prev == Look::Busy || now_look == Look::On)) {
            a.spin = (a.spin + SPIN * dt).rem_euclid(360.0);
            moving |= now_look == Look::Busy;
        }
        moving
    }

    /// Yeni durum: anahtarin hedefi ve simge gecisi.
    fn set_view(&mut self, view: &View) {
        let before = look(&self.view);
        self.view = view.clone();
        self.knob_target = if is_on(view) { 1.0 } else { 0.0 };
        let after = look(view);
        if after != before {
            self.anim.prev = before;
            self.anim.t = 0.0;
        }
        self.wake();
        self.redraw();
    }
}

pub fn is_open() -> bool {
    MENU.with(|m| m.borrow().is_some())
}

/// Acik menuyu yeni durumla yeniler (anahtar kayar, simge gecis yapar).
pub fn update(view: &View) {
    MENU.with(|m| {
        if let Some(menu) = m.borrow_mut().as_mut() {
            menu.set_view(view);
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
                    menu.wake();
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
        // Sinif imleci bos birakilirsa Windows menunun ustunde bekleme imlecini gosterir.
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        };
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

        let anim = Anim::settled(view);
        let mut menu = Menu {
            hwnd,
            surface,
            scale,
            dark,
            view: view.clone(),
            hover: None,
            knob_target: anim.knob,
            anim,
            last_tick: Instant::now(),
            animating: false,
            tracking: false,
            was_active: false,
            on_action,
        };
        menu.surface.render(&menu.view, &menu.anim, dark, scale);
        // Tarama surerken acildiysa yaylar hemen donmeye baslar.
        if look(view) == Look::Busy {
            menu.wake();
        }
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
        WM_TIMER if wparam == TIMER_ANIM => {
            MENU.with(|m| {
                if let Some(menu) = m.borrow_mut().as_mut() {
                    if !menu.step() {
                        menu.animating = false;
                        unsafe { KillTimer(hwnd, TIMER_ANIM) };
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
                KillTimer(hwnd, TIMER_ANIM);
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
    let mut a = Anim::settled(view);
    if let Some(i) = hover {
        a.hover[i] = 1.0;
    }
    render_anim(view, a, dark, scale)
}

/// Testler: simge gecisinin bir karesi (`from` -> view, ilerleme `t`, donus acisi `spin`).
#[cfg(test)]
pub fn render_transition_for_test(view: &View, from: &View, t: f32, spin: f32, dark: bool, scale: f32) -> (usize, usize, Vec<u8>) {
    let mut a = Anim::settled(view);
    a.prev = look(from);
    a.t = t;
    a.spin = spin;
    render_anim(view, a, dark, scale)
}

#[cfg(test)]
fn render_anim(view: &View, a: Anim, dark: bool, scale: f32) -> (usize, usize, Vec<u8>) {
    let mut s = Surface::new(scale).expect("yuzey");
    s.render(view, &a, dark, scale);
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
