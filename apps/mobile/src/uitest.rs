//! Telefon arayuzunun testleri: pencere acmadan, yazilim cizicisiyle bellekte (bilgisayarda).
//! Telefonda Skia cizer; yerlesim ayni oldugu icin bu testler duzeni ve gezinmeyi dogrular.
//! `GDPI3_SHOTS=<klasor>` verilirse ekran goruntuleri PNG olarak yazilir.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, Rgb8Pixel, SharedString, VecModel};

use crate::{AppState, AppWindow, Choice, PowerState, Theme};

/// Pixel 7 (412 x 915 dp; uygulama alani gezinme cubugu disinda).
pub const W: f32 = 412.0;
pub const H: f32 = 892.0;
/// Durum cubugu ve hareketli gezinme cubugu (Android 15, kenardan kenara).
pub const TOP: f32 = 32.0;
pub const BOTTOM: f32 = 24.0;

struct TestPlatform {
    window: Rc<MinimalSoftwareWindow>,
    now_ms: Rc<Cell<u64>>,
}

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        Duration::from_millis(self.now_ms.get())
    }
}

pub struct Harness {
    pub app: AppWindow,
    window: Rc<MinimalSoftwareWindow>,
    now_ms: Rc<Cell<u64>>,
    scale: f32,
    width: u32,
    height: u32,
    pixels: Vec<Rgb8Pixel>,
}

impl Harness {
    pub fn new(scale: f32) -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let now_ms = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(TestPlatform { window: window.clone(), now_ms: now_ms.clone() }))
            .expect("platform zaten kurulu");
        let app = AppWindow::new().expect("pencere");
        slint::select_bundled_translation("").expect("ceviri secimi");
        app.set_test_inset_top(TOP);
        app.set_test_inset_bottom(BOTTOM);
        let (width, height) = ((W * scale) as u32, (H * scale) as u32);
        window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        window.set_size(PhysicalSize::new(width, height));
        app.show().expect("goster");
        let mut h = Harness { app, window, now_ms, scale, width, height, pixels: vec![Rgb8Pixel::default(); (width * height) as usize] };
        sample(&h.app);
        h.advance(600);
        h
    }

    /// Zamani 16 ms adimlarla ilerletir; gercek penceredeki gibi her adimda cizer.
    pub fn advance(&mut self, ms: u64) {
        let mut left = ms;
        while left > 0 {
            let step = left.min(16);
            self.now_ms.set(self.now_ms.get() + step);
            slint::platform::update_timers_and_animations();
            self.draw();
            left -= step;
        }
    }

    pub fn tap(&mut self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        self.advance(32);
        self.window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        self.window.dispatch_event(WindowEvent::PointerExited);
        self.advance(500);
    }

    /// Android'in geri tusu (Slint'e `Key.Back` olarak gelir).
    pub fn back(&mut self) {
        let text: SharedString = Key::Back.into();
        self.window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.window.dispatch_event(WindowEvent::KeyReleased { text });
        self.advance(500);
    }

    pub fn page(&self) -> (i32, i32) {
        (self.app.get_depth(), self.app.get_sub())
    }

    fn draw(&mut self) {
        let (w, pixels) = (self.width as usize, &mut self.pixels);
        self.window.draw_if_needed(|r| {
            r.render(pixels, w);
        });
    }

    pub fn pixel(&mut self, x: f32, y: f32) -> (u8, u8, u8) {
        self.draw();
        let (px, py) = ((x * self.scale) as usize, (y * self.scale) as usize);
        let p = self.pixels[py * self.width as usize + px];
        (p.r, p.g, p.b)
    }

    pub fn shot(&mut self, name: &str) {
        self.draw();
        let Some(dir) = std::env::var_os("GDPI3_SHOTS") else { return };
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let rgb: Vec<u8> = self.pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        std::fs::write(dir.join(format!("{name}.png")), png(self.width, self.height, &rgb)).expect("png yaz");
    }
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn choice(id: &str, title: &str, subtitle: &str, badge: &str) -> Choice {
    Choice { id: id.into(), title: title.into(), subtitle: subtitle.into(), badge: badge.into() }
}

/// Telefonda pencere kodunun doldurdugu durum: proxy yontemleri, DNS hazirlari, varsayilan ayar.
fn sample(app: &AppWindow) {
    let st = app.global::<AppState>();
    st.set_mobile(true);
    st.set_power(PowerState::Off);
    st.set_status_title("Kapalı".into());
    st.set_status_sub("Bağlanmak için dokun".into());
    st.set_method_name("TLS kaydı bölme".into());
    st.set_dns_name("Cloudflare DoH".into());
    st.set_version("3.1.3".into());
    let main = vec![
        choice("tlsrec", "TLS kaydı bölme", "İstek, site adının ortasından iki TLS kaydına ayrılır ve iki parça halinde gider. Gecikme eklemez.", "Önerilen"),
        choice("tlsrecdisorder", "TLS kaydı + ters sıra", "TLS kaydı bölmeyle ters sıra birlikte.", ""),
    ];
    let more = vec![
        choice("disorder", "Ters sıra", "", ""),
        choice("disordersni", "Ters sıra + ad bölme", "", ""),
        choice("splitsni", "Ad bölme", "", ""),
        choice("split", "Düz bölme", "", ""),
    ];
    st.set_methods(model(main.iter().chain(more.iter()).cloned().collect()));
    st.set_methods_main(model(main));
    st.set_methods_more(model(more));
    st.set_method_header("ÖNERİLEN".into());
    st.set_method_index(0);
    st.set_dns_providers(model(vec![
        choice("cloudflare", "Cloudflare", "Hızlı, kayıt tutmaz", ""),
        choice("google", "Google", "Yaygın ve kararlı", ""),
        choice("quad9", "Quad9", "Zararlı siteleri engeller", ""),
        choice("adguard", "AdGuard", "Reklamları engeller", ""),
    ]));
    st.set_dns_index(0);
}

/// Sayfa basliklari ve satirlar sistem cubuklarinin arasinda: ust cubugun altinda zemin rengi,
/// ana ekranin alt grubu gezinme cubugunun ustunde biter.
#[test]
fn home_sits_between_the_system_bars() {
    let mut h = Harness::new(1.0);
    h.shot("mobil-ana-koyu");
    assert_eq!(h.pixel(W / 2.0, TOP / 2.0), (0, 0, 0), "durum cubugu alani zemin");
    assert_eq!(h.pixel(W / 2.0, H - BOTTOM / 2.0), (0, 0, 0), "gezinme cubugu alani zemin");
    // Alt grup (Yontem / DNS): 2 x 44 px, alt kenari gezinme cubugunun 20 px ustunde.
    let group_bottom = H - BOTTOM - 20.0;
    assert_ne!(h.pixel(40.0, group_bottom - 10.0), (0, 0, 0), "DNS satiri");
    assert_eq!(h.pixel(40.0, group_bottom + 6.0), (0, 0, 0), "grubun alti zemin");
    // Ayarlar dugmesi durum cubugunun altinda.
    h.tap(30.0, TOP + 26.0);
    assert_eq!(h.page(), (1, 0), "ayarlar menusu acilmali");
}

/// Geri tusu: alt sayfadan menuye, menuden ana ekrana; ana ekrandan dogrudan acilan alt sayfadan
/// ana ekrana. Ana ekranda tus uygulamaya birakilir (Android etkinligi kapatir).
#[test]
fn back_key_walks_back() {
    let mut h = Harness::new(1.0);
    h.tap(30.0, TOP + 26.0);
    assert_eq!(h.page(), (1, 0));
    // Menuden "Baglanti yontemi" (satirin yeri yazi tipine gore degisir; dogrudan acilir).
    h.app.set_sub(1);
    h.app.set_depth(2);
    h.advance(500);
    h.back();
    assert_eq!(h.page(), (1, 1));
    h.back();
    assert_eq!(h.page().0, 0);
    h.back();
    assert_eq!(h.page().0, 0, "ana ekranda gezinme degismez");

    // Ana ekrandaki DNS satiri -> DNS sayfasi -> geri: dogrudan ana ekran.
    h.tap(W / 2.0, H - BOTTOM - 20.0 - 22.0);
    assert_eq!(h.page(), (2, 2));
    h.back();
    assert_eq!(h.page().0, 0);
}

/// Android 14+'ta tek basis hem tus olayi hem geri hareketi olarak arka arkaya gelir: tek ekran
/// geri gidilir ve ikinci kopya ana ekrana donulmusse uygulamayi kapatmaz (kabul edilir).
#[test]
fn back_key_delivered_twice_moves_one_screen() {
    let mut h = Harness::new(1.0);
    h.app.set_depth(1);
    h.app.set_sub(5);
    h.app.set_depth(2);
    h.advance(600);
    let text: SharedString = Key::Back.into();
    let twice = |h: &mut Harness| {
        for _ in 0..2 {
            h.window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            h.window.dispatch_event(WindowEvent::KeyReleased { text: text.clone() });
            h.advance(16);
        }
        h.advance(500);
    };
    twice(&mut h);
    assert_eq!(h.page(), (1, 5), "alt sayfadan menuye, bir ekran");
    twice(&mut h);
    assert_eq!(h.page().0, 0, "menuden ana ekrana, bir ekran");
}

/// Metin alani odaktayken (klavye acik) geri tusu once yalnizca odagi birakir; sayfada kalinir.
#[test]
fn back_key_first_leaves_the_text_field() {
    let mut h = Harness::new(1.0);
    h.app.set_depth(1);
    h.app.set_sub(3);
    h.app.set_depth(2);
    h.advance(600);
    // Siteler: "KENDI SITELERIM" alani.
    h.tap(150.0, 266.0);
    h.back();
    assert_eq!(h.page(), (2, 3), "ilk geri yalnizca klavyeyi kapatir");
    h.back();
    assert_eq!(h.page(), (1, 3), "ikinci geri sayfadan cikar");
}

/// Her sayfanin telefon goruntusu (koyu ve acik); GDPI3_SHOTS ile incelenir.
#[test]
fn every_page_renders_on_the_phone() {
    let mut h = Harness::new(1.0);
    for dark in [true, false] {
        h.app.global::<Theme>().set_dark(dark);
        let mode = if dark { "koyu" } else { "acik" };
        h.app.set_depth(0);
        h.advance(600);
        h.shot(&format!("mobil-{mode}-0-ana"));
        h.app.set_depth(1);
        h.advance(600);
        h.shot(&format!("mobil-{mode}-1-ayarlar"));
        for (sub, name) in [(1, "yontem"), (2, "dns"), (3, "siteler"), (4, "genel"), (5, "gorunum"), (6, "hakkinda")] {
            h.app.set_sub(sub);
            h.app.set_depth(2);
            h.advance(600);
            h.shot(&format!("mobil-{mode}-2-{sub}-{name}"));
        }
    }
}

// ---------------------------------------------------------------- en kucuk PNG yazici

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Sikistirmasiz (stored) deflate bloklariyla RGB PNG.
fn png(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for row in rgb.chunks((width * 3) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(if i + 1 == blocks.len() { 1 } else { 0 });
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
