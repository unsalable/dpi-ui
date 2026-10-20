//! Surum imzalama araci. Uygulamayla dagitilmaz; yalnizca surum yayinlayan bilgisayarda calisir.
//!
//!   release-tool keygen <anahtar.pk8>
//!       Yeni Ed25519 anahtari uretir. Ozel anahtar dosyaya yazilir (depoya KOYMAYIN, yedekleyin),
//!       acik anahtar `shared::update::PUBLIC_KEY` icine yapistirilacak bicimde yazdirilir.
//!   release-tool sign <anahtar.pk8> <kurulum.exe> <surum> <cikti-klasoru> [--legacy]
//!       update.json (surum, boyut, SHA-256) ve update.json.sig uretir. --legacy: manifest
//!       paketi eski adla (GoodbyeDPI-UI-3-Setup.exe) gosterir; 3.0.0-alpha.1 de guncellenebilsin.
//!   release-tool verify <klasor>
//!       Klasordeki update.json + imza + kurulum dosyasini uygulamanin gomulu anahtariyla dogrular.

use std::path::{Path, PathBuf};

use ring::rand::SystemRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use shared::update::{self, Manifest, Version};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") if args.len() == 2 => keygen(Path::new(&args[1])),
        Some("sign") if args.len() == 5 || (args.len() == 6 && args[5] == "--legacy") => {
            sign(Path::new(&args[1]), Path::new(&args[2]), &args[3], Path::new(&args[4]), args.len() == 6)
        }
        Some("verify") if args.len() == 2 => verify(Path::new(&args[1]), &update::PUBLIC_KEY),
        _ => Err("kullanım: release-tool keygen <anahtar> | sign <anahtar> <kurulum.exe> <sürüm> <klasör> | verify <klasör>".into()),
    };
    if let Err(e) = result {
        eprintln!("hata: {e}");
        std::process::exit(1);
    }
}

fn keygen(out: &Path) -> Result<(), String> {
    if out.exists() {
        return Err(format!("{} zaten var; üzerine yazılmaz", out.display()));
    }
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| "anahtar üretilemedi")?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| "anahtar okunamadı")?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(out, pkcs8.as_ref()).map_err(|e| e.to_string())?;
    let pk = pair.public_key().as_ref();
    println!("Özel anahtar: {}", out.display());
    println!("Açık anahtar (shared/src/update.rs PUBLIC_KEY):");
    println!("[{}]", pk.iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(", "));
    Ok(())
}

fn sha256_file(path: &Path) -> Result<(String, u64), String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((update::to_hex(ring::digest::digest(&ring::digest::SHA256, &data).as_ref()), data.len() as u64))
}

fn sign(key: &Path, installer: &Path, version: &str, out: &Path, legacy: bool) -> Result<(), String> {
    let v = Version::parse(version).ok_or("sürüm biçimi geçersiz (örn. 3.0.0-alpha.2)")?;
    let pkcs8 = std::fs::read(key).map_err(|e| format!("{}: {e}", key.display()))?;
    let pair = Ed25519KeyPair::from_pkcs8(&pkcs8).map_err(|_| "özel anahtar geçersiz")?;
    if pair.public_key().as_ref() != update::PUBLIC_KEY {
        return Err("bu anahtar uygulamaya gömülü açık anahtarla eşleşmiyor".into());
    }
    let (sha256, size) = sha256_file(installer)?;
    let file = if legacy { update::LEGACY_INSTALLER_NAME } else { update::INSTALLER_NAME };
    let manifest = Manifest { version: v.to_string(), file: file.into(), size, sha256 };
    manifest.check(&v)?;
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    let sig = pair.sign(&json);
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join(update::MANIFEST_NAME), &json).map_err(|e| e.to_string())?;
    std::fs::write(out.join(update::SIGNATURE_NAME), update::to_hex(sig.as_ref())).map_err(|e| e.to_string())?;
    println!("İmzalandı: {} {} bayt, SHA-256 {}", manifest.version, manifest.size, manifest.sha256);
    verify(out, &update::PUBLIC_KEY)
}

/// Uygulamanin yapacagi dogrulamanin aynisi (yayindan once son kontrol).
fn verify(dir: &Path, public_key: &[u8; 32]) -> Result<(), String> {
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let p: PathBuf = dir.join(name);
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let json = read(update::MANIFEST_NAME)?;
    let sig = update::from_hex(&String::from_utf8_lossy(&read(update::SIGNATURE_NAME)?)).ok_or("imza onaltılık değil")?;
    UnparsedPublicKey::new(&ED25519, public_key).verify(&json, &sig).map_err(|_| "imza geçersiz")?;
    let m: Manifest = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    let v = Version::parse(&m.version).ok_or("manifest sürümü geçersiz")?;
    m.check(&v)?;
    let installer = dir.join(&m.file);
    if installer.exists() {
        let (sha, size) = sha256_file(&installer)?;
        if sha != m.sha256 || size != m.size {
            return Err("kurulum dosyası manifestle eşleşmiyor".into());
        }
    }
    println!("Doğrulandı: {} ({})", m.version, if installer.exists() { "kurulum dosyası eşleşti" } else { "kurulum dosyası klasörde yok" });
    Ok(())
}
