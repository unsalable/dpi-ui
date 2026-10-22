//! Arayuz testleri: pencere acmadan, yazilim cizicisiyle bellekte calisir. Zaman elle
//! ilerletilir; boylece animasyon kareleri ve tiklamalar her seferinde ayni sonucu verir.
//! `GDPI3_SHOTS=<klasor>` verilirse ekran goruntuleri PNG olarak yazilir.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, Rgb8Pixel};

use crate::{AppState, AppWindow, PowerState, Theme};

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
    width: u32,
    height: u32,
    pixels: Vec<Rgb8Pixel>,
}

pub const W: f32 = 400.0;
pub const H: f32 = 640.0;

impl Harness {
    /// Her test kendi is parcaciginda calisir; Slint baglami is parcacigina ozeldir.
    pub fn new(scale: f32) -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        let now_ms = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(TestPlatform { window: window.clone(), now_ms: now_ms.clone() }))
            .expect("platform zaten kurulu");
        let app = AppWindow::new().expect("pencere");
        // Slint ilk bilesende sistem diline gecer; testler her makinede Turkce baslar.
        slint::select_bundled_translation("").expect("ceviri secimi");
        let (width, height) = ((W * scale) as u32, (H * scale) as u32);
        window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
        window.set_size(PhysicalSize::new(width, height));
        app.show().expect("goster");
        let mut h = Harness { app, window, now_ms, width, height, pixels: vec![Rgb8Pixel::default(); (width * height) as usize] };
        h.advance(50);
        h.draw();
        h
    }

    pub fn state(&self) -> AppState<'_> {
        self.app.global::<AppState>()
    }

    pub fn set_dark(&self, dark: bool) {
        self.app.global::<Theme>().set_dark(dark);
    }

    /// Zamani 8 ms adimlarla ilerletir (animasyonlar ve zamanlayicilar calisir).
    pub fn advance(&mut self, ms: u64) {
        let mut left = ms;
        while left > 0 {
            let step = left.min(8);
            self.now_ms.set(self.now_ms.get() + step);
            slint::platform::update_timers_and_animations();
            left -= step;
        }
    }

    pub fn click(&mut self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window.dispatch_event(WindowEvent::PointerMoved { position });
        self.window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
        self.advance(16);
        self.window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        self.advance(16);
    }

    /// Basili tutup `dx` kadar saga ceker, sonra birakir.
    pub fn press_drag(&mut self, x: f32, y: f32, dx: f32) {
        let start = LogicalPosition::new(x, y);
        self.window.dispatch_event(WindowEvent::PointerMoved { position: start });
        self.window.dispatch_event(WindowEvent::PointerPressed { position: start, button: PointerEventButton::Left });
        self.advance(16);
        for i in 1..=4 {
            let position = LogicalPosition::new(x + dx * i as f32 / 4.0, y);
            self.window.dispatch_event(WindowEvent::PointerMoved { position });
            self.advance(16);
        }
        let end = LogicalPosition::new(x + dx, y);
        self.window.dispatch_event(WindowEvent::PointerReleased { position: end, button: PointerEventButton::Left });
        self.advance(16);
    }

    pub fn hover(&mut self, x: f32, y: f32) {
        self.window.dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(x, y) });
        self.advance(300);
    }

    /// Fare tekerlegi (negatif dy asagi kaydirir).
    pub fn scroll(&mut self, x: f32, y: f32, dy: f32) {
        let position = LogicalPosition::new(x, y);
        self.window.dispatch_event(WindowEvent::PointerMoved { position });
        self.window.dispatch_event(WindowEvent::PointerScrolled { position, delta_x: 0.0, delta_y: dy });
        self.advance(400);
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

    /// (x, y) mantiksal noktasindaki renk.
    pub fn pixel(&mut self, x: f32, y: f32) -> (u8, u8, u8) {
        self.draw();
        let scale = self.width as f32 / W;
        let (px, py) = ((x * scale) as usize, (y * scale) as usize);
        let p = self.pixels[py * self.width as usize + px];
        (p.r, p.g, p.b)
    }

    /// GDPI3_SHOTS ayarliysa PNG yazar.
    pub fn shot(&mut self, name: &str) {
        self.draw();
        let Some(dir) = std::env::var_os("GDPI3_SHOTS") else { return };
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let rgb: Vec<u8> = self.pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        std::fs::write(dir.join(format!("{name}.png")), png(self.width, self.height, &rgb)).expect("png yaz");
    }

    /// Birden cok kareyi yan yana tek goruntude birlestirmek icin ham RGB kopyasi.
    pub fn frame(&mut self) -> Vec<u8> {
        self.draw();
        self.pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect()
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Kareleri yatay seritte birlestirip PNG yazar (animasyon incelemesi icin).
pub fn strip(name: &str, frames: &[Vec<u8>], width: u32, height: u32, crop: (u32, u32, u32, u32)) {
    let Some(dir) = std::env::var_os("GDPI3_SHOTS") else { return };
    let (cx, cy, cw, ch) = crop;
    let total_w = cw * frames.len() as u32;
    let mut out = vec![0u8; (total_w * ch * 3) as usize];
    for (i, f) in frames.iter().enumerate() {
        for y in 0..ch {
            for x in 0..cw {
                let src = (((cy + y) * width + cx + x) * 3) as usize;
                let dst = ((y * total_w + i as u32 * cw + x) * 3) as usize;
                if (cy + y) < height && (cx + x) < width {
                    out[dst..dst + 3].copy_from_slice(&f[src..src + 3]);
                }
            }
        }
    }
    let dir = std::path::PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join(format!("{name}.png")), png(total_w, ch, &out)).expect("png yaz");
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::ipc::EngineState;

    fn counter() -> (Rc<Cell<i32>>, impl Fn() + 'static) {
        let c = Rc::new(Cell::new(0));
        let k = c.clone();
        (c, move || k.set(k.get() + 1))
    }

    /// Yontem sayfasi: saglayicinin yontemleri ustte; digerleri kapali baslar, "daha goster"
    /// satiri acar (kapaliyken altindaki gizli satirlar tiklanmaz), acilan satir dogru yontemi
    /// secer; secili yontem oradaysa liste acik kalir.
    #[test]
    fn method_list_opens_and_selects() {
        use slint::Model;
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        crate::window::config_for_test(&h.app, |c| {
            c.isp = "turktelekom".into();
            c.method = "disorder".into();
        });
        let picked = Rc::new(Cell::new(-1));
        let (p, weak) = (picked.clone(), h.app.as_weak());
        h.state().on_select_method(move |i| {
            p.set(i);
            if let Some(app) = weak.upgrade() {
                app.global::<AppState>().set_method_index(i);
            }
        });
        assert_eq!((h.state().get_methods_main().row_count(), h.state().get_methods_more().row_count()), (4, 12));
        assert_eq!(h.state().get_method_header(), "TÜRK TELEKOM İÇİN");
        h.app.set_depth(2);
        h.app.set_sub(1);
        h.advance(600);
        h.click(200.0, 451.0);
        h.advance(500);
        assert_eq!(picked.get(), -1, "daha goster satiri yontem secmemeli");
        h.click(200.0, 450.0 + 44.0 * 2.0);
        assert_eq!(picked.get(), 6, "acilan listenin ucuncu satiri (MD5 + TTL 3) secilmeli");
        h.advance(500);
        // Secim asagidaki listeye gecince ustteki aciklama kapanir; ikinci satir yukari kayar.
        h.click(200.0, 217.0);
        assert_eq!(picked.get(), 1, "ustteki ikinci satir (Sahte TTL 4) secilmeli");
    }

    /// Secim degisince isaret yeni satirda soldan saga cizilir (bir anda belirmez), eskisi kaybolur.
    #[test]
    fn check_mark_draws_in() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        h.app.set_depth(2);
        h.app.set_sub(2);
        h.advance(600);
        h.frame();
        // DNS listesi: Cloudflare (satir merkezi y=190) secili; isaretin sag ucu (366, y-4).
        let blue = |h: &mut Harness, y: f32| {
            let (r, g, b) = h.pixel(366.0, y - 4.0);
            b > 180 && r < 120 && g > 90
        };
        assert!(blue(&mut h, 190.0) && !blue(&mut h, 247.0));
        h.state().set_dns_index(1);
        let mut drawn_at = None;
        for step in 0..30 {
            h.advance(16);
            h.frame();
            if drawn_at.is_none() && blue(&mut h, 247.0) {
                drawn_at = Some(step * 16 + 16);
            }
        }
        let at = drawn_at.expect("yeni satirda isaret cizilmeli");
        assert!(at >= 120, "isaret cizilerek gelmeli, {at} ms'de tamamlandi (aninda olmamali)");
        assert!(!blue(&mut h, 190.0), "eski satirdaki isaret kaybolmali");
    }

    /// Gercek akis: dokunus ayari servise gonderir, ekran servisin onayladigi ayarla yenilenir.
    /// Liste ayni kaldigi icin satirlar yeniden kurulmamali; isaret cizilerek gelmeli ve secili
    /// yontemin aciklamasi yumusakca acilmali.
    #[test]
    fn selection_animates_after_service_confirms() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        let confirm = |h: &Harness, provider: &str, method: &str| {
            crate::window::config_for_test(&h.app, |c| {
                c.isp = "turktelekom".into();
                c.method = method.into();
                c.dns.provider = provider.into();
            });
        };
        confirm(&h, "cloudflare", "disorder");
        h.app.set_depth(2);
        h.app.set_sub(2);
        h.advance(600);
        h.frame();
        let blue = |h: &mut Harness, y: f32| {
            let (r, g, b) = h.pixel(366.0, y - 4.0);
            b > 180 && r < 120 && g > 90
        };
        assert!(blue(&mut h, 190.0) && !blue(&mut h, 247.0));
        confirm(&h, "google", "disorder");
        let mut drawn_at = None;
        for step in 0..30 {
            h.advance(16);
            h.frame();
            if drawn_at.is_none() && blue(&mut h, 247.0) {
                drawn_at = Some(step * 16 + 16);
            }
        }
        let at = drawn_at.expect("yeni satirda isaret cizilmeli");
        assert!(at >= 120, "servis onayindan sonra isaret cizilerek gelmeli, {at} ms'de tamamlandi");
        assert!(!blue(&mut h, 190.0), "eski satirdaki isaret kaybolmali");

        // Yontem: Ters sira (aciklamasi acik) -> Sahte TTL 4; Ters sira satiri bir anda degil
        // yavasca kisalir (ikinci satirin ust kenari kademe kademe yukari kayar).
        h.app.set_sub(1);
        h.advance(600);
        h.frame();
        let row_top = |h: &mut Harness| -> f32 {
            // Sahte TTL 4 satirinin basligi: y ekseninde ilk acik renkli nokta (x=40).
            let mut y = 200.0;
            while y < 300.0 {
                let (r, _, _) = h.pixel(40.0, y);
                if r > 150 {
                    return y;
                }
                y += 1.0;
            }
            y
        };
        let before = row_top(&mut h);
        confirm(&h, "google", "ttl4");
        h.advance(64);
        let mid = row_top(&mut h);
        h.advance(600);
        let after = row_top(&mut h);
        assert!(after < before - 10.0, "aciklama kapaninca satir yukari kaymali ({before} -> {after})");
        assert!(mid > after + 2.0 && mid < before - 1.0, "kayma yumusak olmali: {before} -> {mid} -> {after}");
    }

    /// Alt sayfadaki bos bir yere tiklamak alttaki sayfaya (menu, ana ekran) gecmemeli.
    /// (Eski surumde "Tesekkurler" kartina tiklamak alttaki menuden "Genel"i aciyordu.)
    #[test]
    fn hidden_pages_never_receive_clicks() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        let (toggles, f) = counter();
        h.state().on_power_toggle(f);
        let mut leaks = Vec::new();
        for sub in 1..=6 {
            for y in (60..630).step_by(23) {
                for x in (20..390).step_by(29) {
                    h.app.set_depth(2);
                    h.app.set_sub(sub);
                    h.advance(500);
                    h.click(x as f32, y as f32);
                    h.advance(400);
                    if h.page() != (2, sub) {
                        leaks.push(format!("alt sayfa {sub}: ({x},{y}) -> {:?}", h.page()));
                    }
                }
            }
        }
        // Menunun bos alt kismi da ana ekrandaki dugmelere gecmemeli.
        for y in (440..630).step_by(17) {
            for x in (20..390).step_by(29) {
                h.app.set_depth(1);
                h.advance(500);
                h.click(x as f32, y as f32);
                h.advance(400);
                if h.app.get_depth() != 1 {
                    leaks.push(format!("menu: ({x},{y}) -> {:?}", h.page()));
                }
            }
        }
        assert_eq!(toggles.get(), 0, "gizli ana ekrandaki baglanti dugmesi tetiklendi");
        assert!(leaks.is_empty(), "{} tiklama alttaki sayfaya gecti, ornek: {:?}", leaks.len(), &leaks[..leaks.len().min(5)]);
    }

    /// Ayarlar dugmesi, menu satirlari ve geri dugmeleri dogru sayfalara gider.
    #[test]
    fn navigation_goes_where_it_says() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        h.click(28.0, 26.0);
        h.advance(500);
        assert_eq!(h.app.get_depth(), 1, "ayarlar dugmesi menuyu acmali");
        for (y, sub) in [(136.0, 1), (180.0, 2), (224.0, 3), (292.0, 4), (336.0, 5), (404.0, 6)] {
            h.click(200.0, y);
            h.advance(500);
            assert_eq!(h.page(), (2, sub), "menu satiri y={y}");
            h.click(40.0, 26.0);
            h.advance(500);
            assert_eq!(h.app.get_depth(), 1, "geri dugmesi menuye donmeli (alt sayfa {sub})");
        }
        h.click(24.0, 26.0);
        h.advance(500);
        assert_eq!(h.app.get_depth(), 0, "menudeki geri dugmesi ana ekrana donmeli");

        // Ana ekrandaki Yontem/DNS satirlari dogrudan alt sayfayi acar; geri ana ekrana doner.
        for (y, sub) in [(554.0, 1), (598.0, 2)] {
            h.click(200.0, y);
            h.advance(500);
            assert_eq!(h.page(), (2, sub));
            h.click(30.0, 26.0);
            h.advance(500);
            assert_eq!(h.app.get_depth(), 0, "ana ekrandan acilan sayfa ana ekrana donmeli");
        }
    }

    /// Ayarlar ve geri dugmelerine hizli hizli basmak pencere suruklemeyi baslatmamali. Gecis
    /// sirasinda dugmeler kaydigi icin tiklamalar ust cubugun bos yerine denk gelir; suruklemede
    /// Windows uygulamanin dongusunu durdurur ve gecis donar.
    #[test]
    fn spamming_nav_buttons_never_starts_window_drag() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        let (drags, f) = counter();
        h.state().on_drag_window(f);
        // Ayarlar/geri dugmesinin yeri; her tiklama bir oncekinin gecisi bitmeden gelir.
        for _ in 0..12 {
            h.click(28.0, 26.0);
            h.advance(40);
        }
        // Menuden alt sayfa acip geri tusuna seri basmak.
        h.advance(600);
        h.app.set_depth(1);
        h.advance(600);
        for _ in 0..6 {
            h.click(200.0, 136.0);
            h.advance(30);
            h.click(28.0, 26.0);
            h.advance(30);
        }
        // Ust cubugun bos yerine tiklamak da surukleme degildir.
        h.advance(600);
        h.click(200.0, 26.0);
        assert_eq!(drags.get(), 0, "tiklamalar pencere suruklemesi baslatmamali");

        // Basili tutup cekmek ise pencereyi surukler (bir kez).
        h.app.set_depth(0);
        h.advance(600);
        h.press_drag(200.0, 26.0, 12.0);
        assert_eq!(drags.get(), 1, "basili tutup cekmek pencereyi suruklemeli");
    }

    /// Olcum (elle calistirilir): sayfa gecisinde bir karenin yazilim cizim suresi.
    /// `cargo test --release -p goodbyedpi3-ui frame_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn frame_cost() {
        // Slint platformu is parcacigi basina bir kez kurulur: her olcek kendi is parcaciginda.
        for scale in [1.0f32, 1.5, 2.0] {
            std::thread::spawn(move || frame_cost_at(scale)).join().unwrap();
        }
    }

    fn frame_cost_at(scale: f32) {
        {
            let mut h = Harness::new(scale);
            gallery::sample(&h, EngineState::On);
            h.advance(800);
            let mut times = Vec::new();
            for (depth, sub) in [(1, 0), (2, 1), (1, 1), (0, 1), (2, 2), (0, 2)] {
                h.app.set_depth(depth);
                h.app.set_sub(sub);
                for _ in 0..26 {
                    h.advance(16);
                    let t = std::time::Instant::now();
                    let _ = h.frame();
                    times.push(t.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let avg = times.iter().sum::<f64>() / times.len() as f64;
            let p95 = times[times.len() * 95 / 100];
            eprintln!("olcek {scale}: kare ort {avg:.2} ms, p95 {p95:.2} ms, en kotu {:.2} ms", times[times.len() - 1]);
        }
    }

    /// Ayarlar dugmesi sol ustte: menu soldan gelir, ana ekran saga kayar; kapanirken menu sola
    /// cekilir. Menuden acilan alt sayfa ise sagdan gelir (menu sola kayar).
    #[test]
    fn settings_menu_slides_in_from_the_left() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        // Gercek pencere gibi her 16 ms'de bir kare cizerek zamani ilerletir.
        let play = |h: &mut Harness, ms: u64, frames: &mut Vec<Vec<u8>>| {
            for _ in 0..ms / 16 {
                h.advance(16);
                frames.push(h.frame());
            }
        };
        let mut open = vec![h.frame()];
        assert!(h.app.get_menu_x() <= -W + 0.5, "kapaliyken menu solda beklemeli: {}", h.app.get_menu_x());

        h.click(28.0, 26.0);
        play(&mut h, 80, &mut open);
        let (m, home) = (h.app.get_menu_x(), h.app.get_home_x());
        assert!(m < 0.0 && m > -W, "acilirken menu soldan gelmeli: {m}");
        assert!(home > 0.0, "acilirken ana ekran saga kaymali: {home}");
        play(&mut h, 560, &mut open);
        assert_eq!(h.app.get_menu_x(), 0.0);
        let (w, hh) = h.size();
        let pick: Vec<Vec<u8>> = [0usize, 2, 4, 6, 9, 14, 24].iter().map(|&i| open[i].clone()).collect();
        strip("settings-open", &pick, w, hh, (0, 0, w, hh));

        // Alt sayfa sagdan gelir; menu sola kayar.
        let mut rest = Vec::new();
        h.click(200.0, 136.0);
        play(&mut h, 80, &mut rest);
        assert!(h.app.get_menu_x() < 0.0, "alt sayfa acilirken menu sola kaymali");
        play(&mut h, 560, &mut rest);
        h.click(40.0, 26.0);
        play(&mut h, 640, &mut rest);
        assert_eq!(h.app.get_menu_x(), 0.0, "alt sayfadan donunce menu yerinde");

        // Kapanirken menu sola cekilir, ana ekran sagdan yerine doner.
        let mut close = vec![h.frame()];
        h.click(24.0, 26.0);
        play(&mut h, 80, &mut close);
        let (m, home) = (h.app.get_menu_x(), h.app.get_home_x());
        assert!(m < 0.0, "kapanirken menu sola cekilmeli: {m}");
        assert!(home > 0.0, "ana ekran sagdan donmeli: {home}");
        play(&mut h, 560, &mut close);
        assert_eq!(h.app.get_home_x(), 0.0);
        assert!(h.app.get_menu_x() <= -W + 0.5);
        let pick: Vec<Vec<u8>> = [0usize, 2, 4, 6, 9, 14, 24].iter().map(|&i| close[i].clone()).collect();
        strip("settings-close", &pick, w, hh, (0, 0, w, hh));
    }

    /// Simgeye dokunmak baglanir/keser; motor hizmeti yoksa hizmeti baslatir.
    #[test]
    fn glyph_toggles_power_or_starts_service() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        let (toggles, f) = counter();
        h.state().on_power_toggle(f);
        let (starts, g) = counter();
        h.state().on_start_service(g);
        h.click(200.0, 258.0);
        assert_eq!((toggles.get(), starts.get()), (1, 0));
        h.state().set_service_missing(true);
        h.click(200.0, 258.0);
        assert_eq!((toggles.get(), starts.get()), (1, 1));
    }

    /// Anahtar satirina tiklamak ters degeri bildirir; ekran degeri yalnizca ayar gelince degisir.
    #[test]
    fn toggle_rows_report_new_value() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        let got = Rc::new(Cell::new(None));
        let g = got.clone();
        h.state().on_set_start_with_windows(move |v| g.set(Some(v)));
        h.app.set_depth(2);
        h.app.set_sub(4);
        h.advance(500);
        assert!(h.state().get_start_with_windows());
        h.click(200.0, 110.0);
        assert_eq!(got.get(), Some(false));
    }

    /// Bildirim gorunur, 3,6 sn sonra kendiliginden kaybolur; yeni bildirim sureyi yeniler.
    #[test]
    fn toast_hides_itself() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        h.state().set_toast("Bir".into());
        h.state().set_toast_serial(1);
        h.advance(100);
        assert!(h.app.get_toast_visible());
        h.advance(3000);
        h.state().set_toast("Iki".into());
        h.state().set_toast_serial(2);
        h.advance(3000);
        assert!(h.app.get_toast_visible(), "yeni bildirim sureyi yenilemeli");
        h.advance(1000);
        assert!(!h.app.get_toast_visible(), "bildirim kendiliginden kaybolmali");
    }

    /// Baglaninca simge yesil daireye, hatada kirmiziya doner.
    #[test]
    fn glyph_colors_follow_state() {
        let mut h = Harness::new(1.0);
        gallery::sample(&h, EngineState::Off);
        h.state().set_power(PowerState::On);
        h.advance(1500);
        let (r, g, b) = h.pixel(141.0, 258.0);
        assert!(g > 150 && r < 120 && b < 140, "bagliyken dairenin sol kenari yesil olmali: {:?}", (r, g, b));
        h.state().set_power(PowerState::Error);
        h.advance(1500);
        let (r, g, _) = h.pixel(143.0, 300.0);
        assert!(r > 200 && g < 120, "hatada kose parantezi kirmizi olmali: {:?}", (r, g));
    }
}

#[cfg(test)]
mod gallery {
    use super::*;
    use shared::ipc::{EngineState, EngineStatus, LearnedHost};

    pub fn engine(state: EngineState) -> EngineStatus {
        EngineStatus { state, message: None, since_ms: None, method_name: "Varsayılan".into(), dns_name: "Cloudflare".into() }
    }

    pub fn sample(h: &Harness, state: EngineState) {
        let learned = vec![LearnedHost { host: "discord.com".into(), reason: "yanıt yok".into(), at_ms: 0 }];
        crate::window::populate_for_test(&h.app, engine(state), learned);
    }

    /// Ozel durumlar: bagli, hata, uyari seridi, bildirim, acilir menu, hiz testi.
    #[test]
    fn states() {
        use crate::BenchItem;
        use slint::{ModelRc, SharedString, VecModel};
        let mut h = Harness::new(1.0);
        sample(&h, EngineState::On);
        h.advance(1500);
        h.shot("state-on");

        h.state().set_banner(SharedString::from("GoodbyeDPI UI (eski sürüm) çalışıyor. Aynı anda iki DPI aracı bağlantıları bozabilir; diğerini kapat."));
        h.state().set_toast(SharedString::from("Vodafone için önerilen yöntem seçildi: Ters sıra."));
        h.state().set_toast_serial(1);
        h.advance(700);
        h.shot("state-banner-toast");

        h.state().set_banner(SharedString::from(""));
        h.state().set_power(PowerState::Error);
        h.state().set_status_title(SharedString::from("Bağlanamadı"));
        h.state().set_status_sub(SharedString::from("Erişim reddedildi. Motor yönetici yetkisiyle çalışmalı (servis kurulu mu?)."));
        h.state().set_toast(SharedString::from("Motor hizmetine bağlanılamadı."));
        h.state().set_toast_error(true);
        h.state().set_toast_serial(2);
        h.advance(1200);
        h.shot("state-error");

        // Sessiz guncelleme: Hakkinda sayfasinda durum, altta pencere kapanmadan onceki bildirim.
        h.state().set_toast_error(false);
        h.state().set_update_text(SharedString::from("3.0.4 yükleniyor…"));
        h.state().set_update_busy(true);
        h.state().set_toast(SharedString::from("Sürüm 3.0.4 yükleniyor; uygulama birkaç saniye içinde yeniden açılacak."));
        h.state().set_toast_serial(3);
        h.app.set_depth(2);
        h.app.set_sub(6);
        h.advance(900);
        h.shot("state-update-installing");
        h.state().set_update_busy(false);
        h.app.set_depth(0);
        h.advance(600);

        h.app.set_depth(2);
        h.app.set_sub(1);
        h.advance(600);
        h.click(200.0, 81.0);
        h.advance(400);
        h.shot("state-isp-menu");
        h.click(200.0, 600.0);
        h.advance(300);

        let bench = vec![
            BenchItem { id: "cloudflare".into(), name: "Cloudflare".into(), ms: 18, ok: true, ratio: 1.0, note: "18 ms".into() },
            BenchItem { id: "google".into(), name: "Google".into(), ms: 24, ok: true, ratio: 0.85, note: "24 ms".into() },
            BenchItem { id: "adguard".into(), name: "AdGuard".into(), ms: 41, ok: true, ratio: 0.45, note: "41 ms".into() },
            BenchItem { id: "quad9".into(), name: "Quad9".into(), ms: 0, ok: false, ratio: 0.0, note: "hata".into() },
        ];
        h.state().set_bench(ModelRc::new(VecModel::from(bench)));
        h.app.set_sub(2);
        h.advance(4000);
        h.shot("state-dns-top");
        h.scroll(200.0, 400.0, -560.0);
        h.shot("state-dns-bench");
        h.scroll(200.0, 400.0, -700.0);
        h.shot("state-dns-bottom");
        h.app.set_sub(3);
        h.advance(600);
        h.scroll(200.0, 400.0, -600.0);
        h.shot("state-sites-bottom");
    }

    /// Simge ayrintilari icin 2x olcekte menu (ust kisim kirpilir).
    #[test]
    fn menu_2x() {
        let mut h = Harness::new(2.0);
        sample(&h, EngineState::Off);
        h.app.set_depth(1);
        h.advance(600);
        let (w, hh) = h.size();
        let f = h.frame();
        strip("menu-2x", &[f], w, hh, (0, 200, 800, 680));
    }

    /// Ust cubuk dugmeleri uzerine gelinmisken (geri, kucult, kapat), 2x olcek.
    #[test]
    fn navbar_hover_2x() {
        let mut h = Harness::new(2.0);
        sample(&h, EngineState::Off);
        let (w, hh) = h.size();
        let mut frames = Vec::new();
        h.app.set_depth(1);
        h.advance(600);
        for x in [28.0, 337.0, 375.0] {
            h.hover(x, 26.0);
            frames.push(h.frame());
        }
        h.app.set_depth(2);
        h.app.set_sub(4);
        h.advance(600);
        h.hover(50.0, 26.0);
        frames.push(h.frame());
        let mut row = Vec::new();
        for f in &frames {
            row.push(f.clone());
        }
        strip("navbar-hover-2x", &row, w, hh, (0, 0, 800, 104));
    }

    /// Her sayfanin koyu ve acik temada goruntusu (GDPI3_SHOTS ayarliysa).
    #[test]
    fn pages() {
        let mut h = Harness::new(1.0);
        sample(&h, EngineState::Off);
        for dark in [true, false] {
            h.set_dark(dark);
            let t = if dark { "dark" } else { "light" };
            h.app.set_depth(0);
            h.advance(600);
            h.shot(&format!("{t}-home"));
            h.app.set_depth(1);
            h.advance(600);
            h.shot(&format!("{t}-menu"));
            for (sub, name) in [(1, "method"), (2, "dns"), (3, "sites"), (4, "general"), (5, "appearance"), (6, "about")] {
                h.app.set_depth(2);
                h.app.set_sub(sub);
                h.advance(600);
                h.shot(&format!("{t}-{name}"));
            }
        }
    }

    /// Yontem ve DNS listeleri bastan sona (Turk Telekom; DNS sifreli ve yonlendirme kipinde).
    #[test]
    fn list_pages() {
        use shared::config::DnsMode;
        let mut h = Harness::new(1.0);
        sample(&h, EngineState::Off);
        crate::window::config_for_test(&h.app, |c| {
            c.isp = "turktelekom".into();
            c.method = "disorder".into();
        });
        h.app.set_depth(2);
        h.app.set_sub(1);
        h.advance(600);
        h.shot("list-method-1");
        h.scroll(200.0, 400.0, -480.0);
        h.shot("list-method-2");
        h.scroll(200.0, 400.0, -2000.0);
        h.shot("list-method-3");
        // Diger yontemleri ac (satir yukaridan asagi acilir), sonra birini sec.
        h.scroll(200.0, 400.0, 3000.0);
        let weak = h.app.as_weak();
        h.state().on_select_method(move |i| {
            if let Some(app) = weak.upgrade() {
                app.global::<AppState>().set_method_index(i);
            }
        });
        h.click(200.0, 451.0);
        let (w, hh) = h.size();
        let mut opening = Vec::new();
        for i in 0..24 {
            if i % 4 == 0 {
                opening.push(h.frame());
            }
            h.frame();
            h.advance(16);
        }
        strip("list-method-opening", &opening, w, hh, (0, 360, w, 280));
        h.advance(200);
        h.shot("list-method-open");
        h.click(200.0, 247.0);
        h.advance(110);
        h.shot("list-method-switching");
        h.advance(500);
        h.shot("list-method-switched");
        h.scroll(200.0, 400.0, -400.0);
        h.shot("list-method-open-2");
        h.set_dark(false);
        h.advance(400);
        h.shot("list-method-light");
        h.set_dark(true);
        h.scroll(200.0, 400.0, 3000.0);
        for (mode, name) in [(DnsMode::Encrypted, "encrypted"), (DnsMode::Redirect, "redirect")] {
            crate::window::config_for_test(&h.app, |c| {
                c.isp = "turktelekom".into();
                c.method = "disorder".into();
                c.dns.mode = mode;
            });
            h.app.set_sub(2);
            h.advance(600);
            h.scroll(200.0, 400.0, 3000.0);
            h.shot(&format!("list-dns-{name}-1"));
            h.scroll(200.0, 400.0, -300.0);
            h.shot(&format!("list-dns-{name}-2"));
        }
    }

    /// Secim isareti kare kare (GDPI3_SHOTS): DNS satiri ve tema secimi, 2x.
    #[test]
    fn check_animation() {
        let mut h = Harness::new(2.0);
        sample(&h, EngineState::Off);
        let (w, hh) = h.size();
        let grab = |h: &mut Harness, name: &str, crop: (u32, u32, u32, u32)| {
            let mut frames = vec![h.frame()];
            for i in 1..=24 {
                h.frame();
                h.advance(16);
                if i % 3 == 0 {
                    frames.push(h.frame());
                }
            }
            strip(name, &frames, w, hh, crop);
        };
        h.app.set_depth(2);
        h.app.set_sub(2);
        h.advance(600);
        h.frame();
        h.state().set_dns_index(1);
        grab(&mut h, "check-dns", (560, 330, 240, 240));
        h.app.set_sub(5);
        h.advance(600);
        h.frame();
        h.state().set_theme_index(1);
        grab(&mut h, "check-theme", (0, 440, 800, 140));
    }

    /// Ingilizce ekranlar (elle calistirilir: surecin dilini degistirdigi icin diger testlerle
    /// ayni anda kosmamali). `GDPI3_SHOTS=<klasor> cargo test -p goodbyedpi3-ui english_pages -- --ignored`
    #[test]
    #[ignore]
    fn english_pages() {
        shared::i18n::set(shared::i18n::Language::English);
        let mut h = Harness::new(1.0);
        slint::select_bundled_translation("en").unwrap();
        sample(&h, EngineState::On);
        h.advance(1500);
        h.shot("en-home");
        h.app.set_depth(1);
        h.advance(600);
        h.shot("en-menu");
        for (sub, name) in [(1, "method"), (2, "dns"), (3, "sites"), (4, "general"), (5, "appearance"), (6, "about")] {
            h.app.set_depth(2);
            h.app.set_sub(sub);
            h.advance(600);
            h.shot(&format!("en-{name}"));
        }
        h.app.set_sub(2);
        h.advance(600);
        h.scroll(200.0, 400.0, -700.0);
        h.shot("en-dns-bottom");
        shared::i18n::set(shared::i18n::Language::Turkish);
    }
}

#[cfg(test)]
mod translations {
    use super::*;
    use std::collections::HashMap;

    /// Acilan tirnaktan sonrasini okur: (metin, kalan).
    fn quoted(s: &str) -> Option<(String, &str)> {
        let mut out = String::new();
        let mut chars = s.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => out.push(match chars.next()?.1 {
                    'n' => '\n',
                    other => other,
                }),
                '"' => return Some((out, &s[i + 1..])),
                c => out.push(c),
            }
        }
        None
    }

    fn screen_texts() -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "slint") {
                let text = std::fs::read_to_string(&path).unwrap();
                let mut rest = text.as_str();
                while let Some(i) = rest.find("@tr(\"") {
                    let (id, after) = quoted(&rest[i + 5..]).expect("kapanmayan @tr");
                    ids.push(id);
                    rest = after;
                }
            }
        }
        ids
    }

    fn english() -> HashMap<String, String> {
        let po = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("lang/en/LC_MESSAGES/goodbyedpi3-ui.po");
        let text = std::fs::read_to_string(po).unwrap();
        let mut map = HashMap::new();
        let mut id = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("msgid \"") {
                id = quoted(rest).map(|(s, _)| s);
            } else if let Some(rest) = line.strip_prefix("msgstr \"") {
                if let (Some(k), Some((v, _))) = (id.take(), quoted(rest)) {
                    map.insert(k, v);
                }
            }
        }
        map
    }

    /// Ekrandaki her `@tr` metninin Ingilizcesi .po dosyasinda olmali (yoksa Ingilizce
    /// arayuzde Turkce kalir).
    #[test]
    fn every_screen_text_has_english() {
        let en = english();
        let ids = screen_texts();
        assert!(ids.len() > 50, "@tr metinleri bulunamadi ({})", ids.len());
        let missing: Vec<&String> = ids.iter().filter(|i| en.get(*i).is_none_or(|v| v.is_empty())).collect();
        assert!(missing.is_empty(), "Ingilizcesi eksik: {missing:?}");
        let unused: Vec<&String> = en.keys().filter(|k| !k.is_empty() && !ids.contains(k)).collect();
        assert!(unused.is_empty(), ".po dosyasinda artik kullanilmayan metinler: {unused:?}");
    }

    /// Ceviri programa gomulu ve calisirken degisir.
    #[test]
    fn english_is_bundled_and_switchable() {
        let h = Harness::new(1.0);
        assert_eq!(h.state().get_status_title(), "Kapalı");
        slint::select_bundled_translation("en").unwrap();
        assert_eq!(h.state().get_status_title(), "Off");
        assert_eq!(h.state().get_update_text(), "Not checked yet");
        slint::select_bundled_translation("").unwrap();
        assert_eq!(h.state().get_status_title(), "Kapalı");
    }
}

#[cfg(test)]
mod layout {
    use super::*;

    slint::slint! {
        import { CheckRow, ToggleRow } from "ui/widgets.slint";
        export component Probe inherits Window {
            width: 400px;
            height: 300px;
            out property <length> long-check: r1.preferred-height;
            out property <length> short-toggle: r2.preferred-height;
            VerticalLayout {
                alignment: start;
                r1 := CheckRow { title: "Baslik"; subtitle: "uzun bir alt satir uzun bir alt satir uzun bir alt satir uzun bir alt satir"; }
                r2 := ToggleRow { title: "Baslik"; }
            }
        }
    }

    /// Satirlar icerige gore uzar (alt yazi tasip alttaki satirin ustune binmez).
    #[test]
    fn rows_grow_with_content() {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform { window: window.clone(), now_ms: Rc::new(Cell::new(0)) })).unwrap();
        let p = Probe::new().unwrap();
        window.set_size(PhysicalSize::new(400, 300));
        p.show().unwrap();
        assert!(p.get_long_check() > 60.0, "iki satirlik alt yazi: {}", p.get_long_check());
        assert!(p.get_short_toggle() >= 44.0, "en az 44 px dokunma alani: {}", p.get_short_toggle());
    }
}

#[cfg(test)]
mod icons {
    use crate::iconart::{Mark, render_bgra};

    /// Simge sayfasi: her boyut 4 kat buyutulmus (piksel piksel incelemek icin), koyu ve
    /// acik gorev cubugu zemininde.
    #[test]
    fn icon_sheet() {
        let Some(dir) = std::env::var_os("GDPI3_SHOTS") else { return };
        let rows: [(Mark, &[u32]); 5] = [
            (Mark::App, &[16, 24, 32, 48, 64]),
            (Mark::On, &[16, 20, 24, 32]),
            (Mark::Off, &[16, 20, 24, 32]),
            (Mark::Busy, &[16, 20, 24, 32]),
            (Mark::Error, &[16, 20, 24, 32]),
        ];
        let (cell, zoom) = (64 * 4 + 16, 4u32);
        let (w, h) = (cell * 5 * 2, cell * rows.len() as u32);
        let mut img = vec![0u8; (w * h * 3) as usize];
        for half in 0..2u32 {
            let bg = if half == 0 { [32u8, 32, 32] } else { [243u8, 243, 243] };
            for y in 0..h {
                for x in half * w / 2..(half + 1) * w / 2 {
                    let i = ((y * w + x) * 3) as usize;
                    img[i..i + 3].copy_from_slice(&bg);
                }
            }
            for (ri, (mark, sizes)) in rows.iter().enumerate() {
                for (ci, &size) in sizes.iter().enumerate() {
                    let px = render_bgra(size, *mark);
                    let z = if size >= 64 { 4 } else { zoom.min(256 / size) };
                    let ox = half * w / 2 + ci as u32 * cell + 8;
                    let oy = ri as u32 * cell + 8;
                    for y in 0..size * z {
                        for x in 0..size * z {
                            let s = (((y / z) * size + x / z) * 4) as usize;
                            let a = px[s + 3] as f32 / 255.0;
                            let d = (((oy + y) * w + ox + x) * 3) as usize;
                            for c in 0..3 {
                                let src = px[s + 2 - c] as f32;
                                img[d + c] = (src * a + img[d + c] as f32 * (1.0 - a)) as u8;
                            }
                        }
                    }
                }
            }
        }
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(std::path::Path::new(&dir).join("icons.png"), super::png(w, h, &img)).unwrap();
    }
}

#[cfg(test)]
mod tray_menu {
    use crate::traymenu::{self, Action, View};
    use shared::ipc::EngineState;

    fn view(connected: bool, state: EngineState) -> View {
        View { connected, state, method: "Ters sıra".into(), dns: "Cloudflare".into(), message: None }
    }

    /// Satirlar dogru eylemlere gider; hizmet yoksa ilk satir hizmeti baslatir.
    #[test]
    fn rows_map_to_actions() {
        let on = view(true, EngineState::On);
        assert_eq!(traymenu::hit_for_test(130.0, 90.0), Some(0));
        assert_eq!(traymenu::hit_for_test(130.0, 135.0), Some(1));
        assert_eq!(traymenu::hit_for_test(130.0, 170.0), Some(2));
        assert_eq!(traymenu::hit_for_test(130.0, 30.0), None, "ust bilgi alani tiklanmaz");
        assert_eq!(traymenu::hit_for_test(130.0, 113.0), None, "ayirici tiklanmaz");
        assert_eq!(traymenu::action_for_test(0, &on), Action::Toggle);
        assert_eq!(traymenu::action_for_test(1, &on), Action::Open);
        assert_eq!(traymenu::action_for_test(2, &on), Action::Quit);
        assert_eq!(traymenu::action_for_test(0, &view(false, EngineState::Off)), Action::StartService);
    }

    /// Menunun gorunumu (GDPI3_SHOTS ayarliysa PNG).
    #[test]
    fn menu_shots() {
        let Some(dir) = std::env::var_os("GDPI3_SHOTS") else { return };
        let cases = [
            ("on", view(true, EngineState::On), None),
            ("off-hover", view(true, EngineState::Off), Some(1)),
            ("error", view(true, EngineState::Error), Some(0)),
            ("missing", view(false, EngineState::Off), None),
        ];
        for scale in [1.0f32, 2.0] {
            let mut frames = Vec::new();
            let (mut fw, mut fh) = (0, 0);
            for dark in [true, false] {
                for (_, v, hover) in &cases {
                    let (w, h, bgra) = traymenu::render_for_test(v, *hover, dark, scale);
                    (fw, fh) = (w, h);
                    frames.push(bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect::<Vec<u8>>());
                }
            }
            // 4 sutun x 2 satir (koyu ustte, acik altta), aralarda 12 px gri bosluk.
            let gap = 12;
            let (cols, rows) = (cases.len(), 2);
            let (tw, th) = (cols * (fw + gap) + gap, rows * (fh + gap) + gap);
            let mut img = vec![90u8; tw * th * 3];
            for (k, f) in frames.iter().enumerate() {
                let (cx, cy) = (gap + (k % cols) * (fw + gap), gap + (k / cols) * (fh + gap));
                for y in 0..fh {
                    let d = ((cy + y) * tw + cx) * 3;
                    img[d..d + fw * 3].copy_from_slice(&f[y * fw * 3..(y + 1) * fw * 3]);
                }
            }
            let _ = std::fs::create_dir_all(&dir);
            let name = format!("tray-menu-{}x.png", scale as u32);
            std::fs::write(std::path::Path::new(&dir).join(name), super::png(tw as u32, th as u32, &img)).unwrap();
        }
    }
}

/// Baglanti simgesi: hareket, durus ve kare kare goruntu.
#[cfg(test)]
mod glyph {
    use super::*;
    use shared::ipc::EngineState;
    use std::io::Write;

    fn set(h: &Harness, power: PowerState, title: &str, sub: &str) {
        h.state().set_power(power);
        h.state().set_status_title(title.into());
        h.state().set_status_sub(sub.into());
    }

    fn apply(h: &Harness, p: PowerState) {
        match p {
            PowerState::Off => set(h, p, "Kapalı", "Bağlanmak için dokun"),
            PowerState::Connecting => set(h, p, "Bağlanıyor…", ""),
            PowerState::On => set(h, p, "Korumalı", ""),
            PowerState::Error => set(h, p, "Bağlanamadı", "Motor başlatılamadı. Yeniden denemek için dokun."),
        }
    }

    fn harness(scale: f32) -> Harness {
        let mut h = Harness::new(scale);
        gallery::sample(&h, EngineState::Off);
        run(&mut h, 600);
        h
    }

    /// Gercek pencere gibi ilerler: her 16 ms'de (gerekiyorsa) cizer. Cizilmeden ilerleyen
    /// zamanda durum gecislerinin baslangic ani kayar (Slint durumu ilk okundugunda degil,
    /// bagimliligi degistiginde zamanlar; okunmamis eski bir isaret gecisi "bitmis" sayar).
    fn run(h: &mut Harness, ms: u64) {
        let mut left = ms;
        while left > 0 {
            h.frame();
            let step = left.min(16);
            h.advance(step);
            left -= step;
        }
        h.frame();
    }

    /// Kareleri satir satir izgaraya dizer (uzun seritler okunmuyor).
    fn grid(name: &str, frames: &[Vec<u8>], width: u32, height: u32, crop: (u32, u32, u32, u32), cols: usize) {
        for (i, row) in frames.chunks(cols).enumerate() {
            strip(&format!("{name}-{i}"), row, width, height, crop);
        }
    }

    /// Simgenin en soldaki boyali noktasi (y satirinda, mantiksal px).
    fn left_edge(h: &mut Harness, y: f32) -> f32 {
        let mut x = 90.0;
        while x < 200.0 {
            let (r, g, b) = h.pixel(x, y);
            if r.max(g).max(b) > 60 {
                return x;
            }
            x += 0.5;
        }
        x
    }

    /// Baglanirken parcalar doner; animasyon bitince cizim durur (bosta islemci harcanmaz).
    #[test]
    fn glyph_moves_while_connecting_and_idles_after() {
        let mut h = harness(1.0);
        apply(&h, PowerState::Connecting);
        run(&mut h, 800);
        let a = h.frame();
        run(&mut h, 120);
        assert!(a != h.frame(), "baglanirken simge hareket etmeli");
        for p in [PowerState::On, PowerState::Off] {
            apply(&h, p);
            run(&mut h, 1200);
        }
        run(&mut h, 2500);
        let a = h.frame();
        run(&mut h, 500);
        assert!(a == h.frame(), "durgun ekran degismemeli");
        assert!(!h.window.window().has_active_animations(), "animasyon kalmamali");
    }

    /// Kesilip donunce parantezler hep duz durur (parcalar koselerine oturur).
    #[test]
    fn glyph_brackets_land_square() {
        let mut h = harness(1.0);
        let rest = h.frame();
        for wait in [700u64, 1333, 2100] {
            apply(&h, PowerState::Connecting);
            run(&mut h, wait);
            apply(&h, PowerState::On);
            run(&mut h, 1700);
            apply(&h, PowerState::Off);
            run(&mut h, 2500);
            assert!(rest == h.frame(), "{wait} ms taramadan sonra simge ayni pozda durmali");
            apply(&h, PowerState::Connecting);
            run(&mut h, wait);
            apply(&h, PowerState::Off);
            run(&mut h, 2500);
            assert!(rest == h.frame(), "{wait} ms sonra vazgecince simge ayni pozda durmali");
        }
    }

    /// Uzerine gelince simge buyur, basinca kuculur.
    #[test]
    fn glyph_hover_and_press() {
        let mut h = harness(2.0);
        h.window.dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(330.0, 120.0) });
        run(&mut h, 500);
        let rest = left_edge(&mut h, 258.0);
        h.window.dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(200.0, 258.0) });
        run(&mut h, 500);
        let hover = left_edge(&mut h, 258.0);
        h.window.dispatch_event(WindowEvent::PointerPressed { position: LogicalPosition::new(200.0, 258.0), button: PointerEventButton::Left });
        run(&mut h, 500);
        let press = left_edge(&mut h, 258.0);
        assert!(hover < rest - 1.0, "uzerine gelince buyumeli ({rest} -> {hover})");
        assert!(press > rest + 1.0, "basinca kuculmeli ({rest} -> {press})");
    }

    /// Kare kare izgara (GDPI3_SHOTS): baglaniyor -> bagli -> kapali -> hata.
    #[test]
    fn connect_animation() {
        let mut h = harness(2.0);
        let (w, hh) = h.size();
        let crop = (192, 300, 416, 432);
        let grab = |h: &mut Harness, name: &str, steps: &[u64]| {
            let mut frames = Vec::new();
            for &ms in steps {
                run(h, ms);
                frames.push(h.frame());
            }
            grid(&format!("anim-{name}"), &frames, w, hh, crop, 6);
        };
        apply(&h, PowerState::Connecting);
        grab(&mut h, "1-connecting", &[0, 40, 40, 40, 40, 60, 60, 80, 100, 150, 200, 300]);
        apply(&h, PowerState::On);
        grab(&mut h, "2-on", &[0, 50, 50, 50, 50, 50, 60, 60, 80, 100, 150, 400]);
        apply(&h, PowerState::Off);
        grab(&mut h, "3-off", &[0, 50, 50, 60, 60, 80, 100, 120, 160, 200, 300, 600]);
        apply(&h, PowerState::Connecting);
        run(&mut h, 900);
        apply(&h, PowerState::Error);
        grab(&mut h, "4-error", &[0, 60, 60, 60, 60, 80, 80, 100, 120, 160, 200, 500]);
    }

    #[derive(Clone, Copy)]
    enum Ev {
        Hover(bool),
        Press,
        Release(PowerState),
        Set(PowerState),
    }

    /// 60 kare/sn ham kareler (GDPI3_FRAMES=<klasor>): <klasor>/connect.raw (RGB, simge ve
    /// yazilar, 2x) + connect.json. Animasyonu video olarak incelemek icin; kare basina cizim
    /// suresini de yazar. `cargo test -p goodbyedpi3-ui connect_video -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn connect_video() {
        let Some(dir) = std::env::var_os("GDPI3_FRAMES") else { return };
        let dir = std::path::PathBuf::from(dir);
        let mut h = harness(2.0);
        let (w, _) = h.size();
        // Mantiksal 40..360 x 130..460.
        let (cx, cy, cw, ch) = (80u32, 260u32, 640u32, 660u32);
        let script: &[(u64, Ev)] = &[
            (250, Ev::Hover(true)),
            (600, Ev::Press),
            (700, Ev::Release(PowerState::Connecting)),
            (2100, Ev::Set(PowerState::On)),
            (3500, Ev::Hover(false)),
            (4100, Ev::Hover(true)),
            (4400, Ev::Press),
            (4500, Ev::Release(PowerState::Off)),
            (5500, Ev::Press),
            (5600, Ev::Release(PowerState::Connecting)),
            (6700, Ev::Set(PowerState::Error)),
            (7900, Ev::Hover(false)),
            (8500, Ev::Set(PowerState::Off)),
        ];
        let end = 9300u64;
        let _ = std::fs::create_dir_all(&dir);
        let mut out = std::io::BufWriter::new(std::fs::File::create(dir.join("connect.raw")).unwrap());
        let center = LogicalPosition::new(200.0, 258.0);
        let away = LogicalPosition::new(330.0, 120.0);
        h.window.dispatch_event(WindowEvent::PointerMoved { position: away });
        let (mut next, mut count, mut t_ms) = (0usize, 0u32, 0f64);
        let mut costs = Vec::new();
        while (t_ms as u64) < end {
            while next < script.len() && script[next].0 <= t_ms as u64 {
                match script[next].1 {
                    Ev::Hover(on) => h.window.dispatch_event(WindowEvent::PointerMoved { position: if on { center } else { away } }),
                    Ev::Press => h.window.dispatch_event(WindowEvent::PointerPressed { position: center, button: PointerEventButton::Left }),
                    Ev::Release(p) => {
                        h.window.dispatch_event(WindowEvent::PointerReleased { position: center, button: PointerEventButton::Left });
                        apply(&h, p);
                    }
                    Ev::Set(p) => apply(&h, p),
                }
                next += 1;
            }
            let started = std::time::Instant::now();
            let f = h.frame();
            costs.push(started.elapsed().as_secs_f64() * 1000.0);
            for y in cy..cy + ch {
                let start = ((y * w + cx) * 3) as usize;
                out.write_all(&f[start..start + (cw * 3) as usize]).unwrap();
            }
            count += 1;
            let before = t_ms as u64;
            t_ms += 1000.0 / 60.0;
            h.advance(t_ms as u64 - before);
        }
        out.flush().unwrap();
        costs.sort_by(|a, b| a.total_cmp(b));
        let avg = costs.iter().sum::<f64>() / costs.len() as f64;
        eprintln!("kare basina cizim ort {avg:.2} ms, %95 {:.2} ms, en kotu {:.2} ms (2x, tum pencere)", costs[costs.len() * 95 / 100], costs[costs.len() - 1]);
        let meta = format!("{{\"width\": {cw}, \"height\": {ch}, \"frames\": {count}, \"fps\": 60}}");
        std::fs::write(dir.join("connect.json"), meta).unwrap();
    }
}
