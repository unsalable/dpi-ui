//! Uygulama isaretinin kodla cizimi: ustu acik bir guc halkasi + dikey cizgi.
//! Hem derleme betigi (exe simgesi .ico) hem calisma ani (tepsi simgeleri) kullanir;
//! boylece ayri resim dosyasi tasinmaz. Kenarlar 4x4 alt ornekleme ile yumusatilir.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    /// Uygulama simgesi: koyu yuvarlak kare zemin + nane->camgobegi halka.
    App,
    /// Tepsi: bagli (nane->camgobegi), saydam zemin.
    On,
    /// Tepsi: kapali (gri).
    Off,
    /// Tepsi: baglaniyor (camgobegi).
    Busy,
    /// Tepsi: hata (mercan).
    Error,
}

type Rgb = (f32, f32, f32);

const MINT: Rgb = (0x2C as f32, 0xF0 as f32, 0xA8 as f32);
const CYAN: Rgb = (0x19 as f32, 0xC3 as f32, 0xF0 as f32);
const GRAY: Rgb = (0x9A as f32, 0xA3 as f32, 0xB5 as f32);
const CORAL: Rgb = (0xFF as f32, 0x5D as f32, 0x6C as f32);
const INK: Rgb = (0x0B as f32, 0x0E as f32, 0x13 as f32);
const INK_EDGE: Rgb = (0x1A as f32, 0x1F as f32, 0x2B as f32);

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

/// `size` x `size` BGRA (on carpimsiz, satir satir yukaridan asagi) piksel dizisi.
pub fn render_bgra(size: u32, mark: Mark) -> Vec<u8> {
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let app = mark == Mark::App;
    // Halka geometrisi (birim kare icinde).
    let (cx, cy) = (0.5, 0.53);
    let (radius, width) = if app { (0.27, 0.085) } else { (0.36, 0.13) };
    let gap = 0.62f32; // ust bosluk acisi (radyan, yarim)
    let bar_w = width * 0.95;
    let bar_top = if app { 0.17 } else { 0.04 };
    let bar_bottom = cy - radius * 0.15;
    let corner = 0.23;

    const SS: u32 = 4;
    for py in 0..size {
        for px in 0..size {
            let (mut bg_cov, mut fg_cov) = (0.0f32, 0.0f32);
            let mut fg_col = (0.0, 0.0, 0.0);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / s;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / s;
                    if app && in_rounded_square(x, y, corner) {
                        bg_cov += 1.0;
                    }
                    let (dx, dy) = (x - cx, y - cy);
                    let d = (dx * dx + dy * dy).sqrt();
                    let ang = dx.atan2(-dy); // 0 = yukari, saat yonunde artar
                    let on_ring = (d - radius).abs() <= width / 2.0 && ang.abs() >= gap;
                    let on_bar = dx.abs() <= bar_w / 2.0 && y >= bar_top && y <= bar_bottom;
                    if on_ring || on_bar {
                        fg_cov += 1.0;
                        // Renk: aciya gore nane -> camgobegi gecisi.
                        let t = ((ang + std::f32::consts::PI) / (2.0 * std::f32::consts::PI)).clamp(0.0, 1.0);
                        let c = match mark {
                            Mark::App | Mark::On => mix(MINT, CYAN, if on_bar { 0.0 } else { t }),
                            Mark::Off => GRAY,
                            Mark::Busy => CYAN,
                            Mark::Error => CORAL,
                        };
                        fg_col.0 += c.0;
                        fg_col.1 += c.1;
                        fg_col.2 += c.2;
                    }
                }
            }
            let n = (SS * SS) as f32;
            let fa = fg_cov / n;
            let ba = bg_cov / n;
            let fc = if fg_cov > 0.0 { (fg_col.0 / fg_cov, fg_col.1 / fg_cov, fg_col.2 / fg_cov) } else { (0.0, 0.0, 0.0) };
            // Zemin: koyu kare, kenara dogru hafif acik (derinlik).
            let bgc = mix(INK, INK_EDGE, 0.35);
            let (r, g, b, a) = if app {
                let a = ba.max(fa);
                if a == 0.0 {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    let c = mix(bgc, fc, if fa > 0.0 { fa / a } else { 0.0 });
                    (c.0, c.1, c.2, a)
                }
            } else {
                (fc.0, fc.1, fc.2, fa)
            };
            let i = ((py * size + px) * 4) as usize;
            out[i] = b as u8;
            out[i + 1] = g as u8;
            out[i + 2] = r as u8;
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

fn in_rounded_square(x: f32, y: f32, r: f32) -> bool {
    let (m0, m1) = (0.04, 0.96);
    if x < m0 || x > m1 || y < m0 || y > m1 {
        return false;
    }
    let cx = x.clamp(m0 + r, m1 - r);
    let cy = y.clamp(m0 + r, m1 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
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
