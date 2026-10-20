//! Uygulama isaretinin kodla cizimi: Face ID'yi andiran kose parantezleri icinde bir dunya.
//! Hem derleme betigi (exe simgesi .ico) hem calisma ani (tepsi simgeleri) kullanir;
//! boylece ayri resim dosyasi tasinmaz. Kenarlar 4x4 alt ornekleme ile yumusatilir.
//! Kucuk boyutlarda (32 px alti) parantezler cizilmez, dunya buyur: ayrinti okunur kalir.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    /// Uygulama simgesi: yesil yuvarlak kare zemin uzerinde beyaz isaret.
    App,
    /// Tepsi: bagli (yesil dunya), saydam zemin.
    On,
    /// Tepsi: kapali (gri).
    Off,
    /// Tepsi: baglaniyor (mavi).
    Busy,
    /// Tepsi: hata (kirmizi).
    Error,
}

type Rgb = (f32, f32, f32);

// Apple sistem renkleri.
const GREEN_TOP: Rgb = (0x43 as f32, 0xD7 as f32, 0x69 as f32);
const GREEN_BOTTOM: Rgb = (0x24 as f32, 0xB0 as f32, 0x4B as f32);
const GREEN: Rgb = (0x30 as f32, 0xD1 as f32, 0x58 as f32);
const GRAY: Rgb = (0x8E as f32, 0x8E as f32, 0x93 as f32);
const BLUE: Rgb = (0x0A as f32, 0x84 as f32, 0xFF as f32);
const RED: Rgb = (0xFF as f32, 0x45 as f32, 0x3A as f32);
const WHITE: Rgb = (255.0, 255.0, 255.0);

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

/// Isaretin geometrisi (birim kare, merkez 0.5).
struct Glyph {
    /// Kose parantezleri: yari kenar, kose yaricapi, kenar ortasindaki bosluk (yari), kalinlik.
    brackets: Option<(f32, f32, f32, f32)>,
    /// Dunya: yaricap, kalinlik, boylam elipsinin yatay yari ekseni, orta boylam cizgisi.
    globe: (f32, f32, f32, bool),
}

fn glyph(size: u32, app: bool) -> Glyph {
    if app && size >= 32 {
        // Arayuzdeki AppMark ile ayni oranlar (84 px karoda 56 px isaret).
        let s = 0.6667;
        Glyph {
            brackets: Some((0.44 * s, 0.13 * s, 0.15 * s, 0.05)),
            globe: (0.22 * s, 0.038, 0.1556 * s, true),
        }
    } else if app {
        Glyph { brackets: None, globe: (0.27, 0.085, 0.12, false) }
    } else {
        Glyph { brackets: None, globe: (0.38, 0.12, 0.17, false) }
    }
}

fn on_glyph(g: &Glyph, x: f32, y: f32) -> bool {
    let (dx, dy) = (x - 0.5, y - 0.5);
    if let Some((half, r, gap, w)) = g.brackets {
        // Yuvarlak koseli karenin cevresi; kenar ortalari bos, uclar yuvarlak.
        let qx = dx.abs() - half + r;
        let qy = dy.abs() - half + r;
        let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
        let sd = outside + qx.max(qy).min(0.0) - r;
        if sd.abs() <= w / 2.0 && dx.abs() >= gap && dy.abs() >= gap {
            return true;
        }
        let caps = [(gap, half), (half, gap)];
        if caps.iter().any(|&(cx, cy)| (dx.abs() - cx).powi(2) + (dy.abs() - cy).powi(2) <= (w / 2.0).powi(2)) {
            return true;
        }
    }
    let (rg, w, a, center_line) = g.globe;
    let d = (dx * dx + dy * dy).sqrt();
    if (d - rg).abs() <= w / 2.0 {
        return true;
    }
    if dy.abs() <= w / 2.0 && dx.abs() <= rg {
        return true;
    }
    if center_line && dx.abs() <= w / 2.0 && dy.abs() <= rg {
        return true;
    }
    // Boylam elipsi (yari eksenler a ve rg); birinci derece uzaklik yaklasimi.
    let k = ((dx / a).powi(2) + (dy / rg).powi(2)).sqrt();
    let grad = ((dx / (a * a)).powi(2) + (dy / (rg * rg)).powi(2)).sqrt();
    grad > 0.0 && ((k - 1.0) * k / grad).abs() <= w / 2.0
}

/// `size` x `size` BGRA (on carpimsiz, satir satir yukaridan asagi) piksel dizisi.
pub fn render_bgra(size: u32, mark: Mark) -> Vec<u8> {
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let app = mark == Mark::App;
    let g = glyph(size, app);
    let tint = match mark {
        Mark::App => WHITE,
        Mark::On => GREEN,
        Mark::Off => GRAY,
        Mark::Busy => BLUE,
        Mark::Error => RED,
    };

    const SS: u32 = 4;
    for py in 0..size {
        for px in 0..size {
            let (mut bg_cov, mut fg_cov) = (0.0f32, 0.0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / s;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / s;
                    if app && in_squircle(x, y) {
                        bg_cov += 1.0;
                    }
                    if on_glyph(&g, x, y) {
                        fg_cov += 1.0;
                    }
                }
            }
            let n = (SS * SS) as f32;
            let (fa, ba) = (fg_cov / n, bg_cov / n);
            let (r, gr, b, a) = if app {
                let a = ba.max(fa);
                if a == 0.0 {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    // Zemin: yukaridan asagi acik yesilden koyu yesile.
                    let bg = mix(GREEN_TOP, GREEN_BOTTOM, (py as f32 + 0.5) / s);
                    let c = mix(bg, WHITE, if fa > 0.0 { (fa / a).min(1.0) } else { 0.0 });
                    (c.0, c.1, c.2, a)
                }
            } else {
                (tint.0, tint.1, tint.2, fa)
            };
            let i = ((py * size + px) * 4) as usize;
            out[i] = b as u8;
            out[i + 1] = gr as u8;
            out[i + 2] = r as u8;
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

/// Surekli egrili yuvarlak kare (superelips, n = 4): arayuzdeki AppMark ile ayni sekil.
fn in_squircle(x: f32, y: f32) -> bool {
    let half = 0.46;
    let (dx, dy) = ((x - 0.5).abs() / half, (y - 0.5).abs() / half);
    dx.powi(4) + dy.powi(4) <= 1.0
}

/// Coklu boyutlu .ico dosyasi (32 bit BMP girisleri).
pub fn build_ico(sizes: &[u32], mark: Mark) -> Vec<u8> {
    let mut images: Vec<Vec<u8>> = Vec::new();
    for &size in sizes {
        let bgra = render_bgra(size, mark);
        let mut bmp = Vec::new();
        // BITMAPINFOHEADER: yukseklik iki kat (renk + maske), alttan yukari satirlar.
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(size as i32).to_le_bytes());
        bmp.extend_from_slice(&((size * 2) as i32).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        let mask_stride = size.div_ceil(32) * 4;
        let image_size = size * size * 4 + mask_stride * size;
        bmp.extend_from_slice(&image_size.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 16]);
        for row in (0..size).rev() {
            let start = (row * size * 4) as usize;
            bmp.extend_from_slice(&bgra[start..start + (size * 4) as usize]);
        }
        bmp.extend(std::iter::repeat_n(0u8, (mask_stride * size) as usize));
        images.push(bmp);
    }
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (i, &size) in sizes.iter().enumerate() {
        let dim = if size >= 256 { 0 } else { size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(images[i].len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += images[i].len() as u32;
    }
    for img in images {
        ico.extend_from_slice(&img);
    }
    ico
}
