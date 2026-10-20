//! Guncelleme denetimi: GitHub surumlerine bakar, yeni surumun kurulum paketini indirir ve
//! imzasini dogrular. Kurulumu BASLATMAZ: hazir oldugunu arayuze bildirir; arayuz kurulumu
//! kullanicinin oturumunda baslatir ve Windows yonetici onayini (UAC) sorar.
//!
//! Dogrulama zinciri: update.json'un Ed25519 imzasi gomulu acik anahtarla dogrulanir ->
//! manifestteki surum yayin etiketiyle ayni ve simdiki surumden yeni olmali -> indirilen
//! paketin boyutu ve SHA-256'si manifestle birebir eslesmeli. Biri tutmazsa dosya silinir.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use ring::signature::{ED25519, UnparsedPublicKey};
use shared::config::{UpdateChannel, UpdateMode};
use shared::update::{self, Candidate, Manifest, Release, UpdateState, Version};

use crate::controller::{Controller, now_ms};
use crate::{https, info, warn};

/// Acilistan sonra ilk denetimden once beklenir (ag baglantisi otursun).
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// Arayuz acilinca istenen denetim bundan yakin zamanda yapildiysa aga cikilmaz.
const MIN_RECHECK_MS: u64 = 10 * 60 * 1000;

pub struct Updater {
    busy: AtomicBool,
}

impl Updater {
    pub fn new() -> Self {
        Self { busy: AtomicBool::new(false) }
    }
}

/// Arka planda duzenli denetim (servis acilisi + her 6 saat).
pub fn spawn_schedule(ctrl: &Arc<Controller>) {
    let weak = Arc::downgrade(ctrl);
    std::thread::Builder::new()
        .name("guncelleme".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            loop {
                let Some(ctrl) = weak.upgrade() else { return };
                run_check(&ctrl, false);
                drop(ctrl);
                std::thread::sleep(CHECK_INTERVAL);
            }
        })
        .ok();
}

/// Arayuzden gelen "simdi denetle" (ya da arayuz acilisi). Is parcacigi baslatir, beklemez.
pub fn request_check(ctrl: &Arc<Controller>, forced: bool) {
    let weak: Weak<Controller> = Arc::downgrade(ctrl);
    std::thread::Builder::new()
        .name("guncelleme-istek".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            if let Some(ctrl) = weak.upgrade() {
                run_check(&ctrl, forced);
            }
        })
        .ok();
}

fn run_check(ctrl: &Arc<Controller>, forced: bool) {
    let config = ctrl.config();
    if config.updates == UpdateMode::Off && !forced {
        return;
    }
    let status = ctrl.update_status();
    if !forced && status.checked_at_ms.is_some_and(|t| now_ms().saturating_sub(t) < MIN_RECHECK_MS) {
        return;
    }
    let updater = ctrl.updater();
    if updater.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    ctrl.set_update(UpdateState::Checking, false);
    let result = match Version::parse(shared::VERSION) {
        Some(current) => check_and_download(&current, config.update_channel == UpdateChannel::Beta, &mut |s| ctrl.set_update(s, false)),
        None => Err("uygulama sürümü okunamadı".into()),
    };
    let state = match result {
        Ok(state) => state,
        Err(e) => {
            warn!("Güncelleme denetlenemedi: {e}");
            UpdateState::Failed { message: e }
        }
    };
    ctrl.set_update(state, true);
    updater.busy.store(false, Ordering::SeqCst);
}

/// Denetler, gerekirse indirir ve dogrular. `progress` ara durumlari (indirme yuzdesi) alir.
pub fn check_and_download(current: &Version, beta: bool, progress: &mut dyn FnMut(UpdateState)) -> Result<UpdateState, String> {
    let current = current.clone();
    let json = https::get(update::RELEASES_API, "application/vnd.github+json", 4 * 1024 * 1024, &mut |_, _| {})?;
    let releases: Vec<Release> = serde_json::from_slice(&json).map_err(|e| format!("sürüm listesi okunamadı: {e}"))?;
    cleanup_installed(&current);
    let Some(candidate) = update::pick(&current, &releases, beta) else {
        return Ok(UpdateState::UpToDate);
    };
    let manifest = fetch_manifest(&candidate)?;
    info!("Yeni sürüm bulundu: {} (şu an {current}).", candidate.version);

    let dir = shared::paths::update_dir();
    crate::store::ensure_update_dir(&dir);
    let installer = shared::paths::update_installer();
    if file_matches(&installer, &manifest) {
        crate::store::secure_update_file(&installer);
        return Ok(ready(&candidate, &manifest));
    }
    let _ = std::fs::remove_file(&installer);

    let version = candidate.version.to_string();
    let mut last = 0u8;
    let data = https::get(&candidate.installer_url, "application/octet-stream", manifest.size, &mut |got, total| {
        let pct = (got * 100 / total.unwrap_or(manifest.size).max(1)).min(100) as u8;
        if pct >= last.saturating_add(5) {
            last = pct;
            progress(UpdateState::Downloading { version: version.clone(), percent: pct });
        }
    })?;
    if data.len() as u64 != manifest.size || sha256(&data) != manifest.sha256 {
        return Err("indirilen paket imzalı manifestle eşleşmiyor; silindi".into());
    }
    let partial = installer.with_extension("partial");
    std::fs::write(&partial, &data).map_err(|e| format!("paket yazılamadı: {e}"))?;
    std::fs::rename(&partial, &installer).map_err(|e| format!("paket yerine konamadı: {e}"))?;
    if !crate::store::secure_update_file(&installer) {
        warn!("Güncelleme paketinin izinleri ayarlanamadı.");
    }
    info!("Sürüm {} indirildi ve doğrulandı.", candidate.version);
    Ok(ready(&candidate, &manifest))
}

fn ready(c: &Candidate, m: &Manifest) -> UpdateState {
    let notes: String = c.notes.chars().take(2000).collect();
    UpdateState::Ready { version: c.version.to_string(), notes, sha256: m.sha256.clone() }
}

/// Imzayi dogrular ve manifesti adaya karsi denetler.
fn fetch_manifest(c: &Candidate) -> Result<Manifest, String> {
    let json = https::get(&c.manifest_url, "application/octet-stream", 16 * 1024, &mut |_, _| {})?;
    let sig_text = https::get(&c.signature_url, "application/octet-stream", 1024, &mut |_, _| {})?;
    verify_manifest(&json, &sig_text, &update::PUBLIC_KEY, &c.version)
}

pub fn verify_manifest(json: &[u8], sig_text: &[u8], key: &[u8; 32], expected: &Version) -> Result<Manifest, String> {
    let sig = update::from_hex(&String::from_utf8_lossy(sig_text)).ok_or("imza dosyası bozuk")?;
    UnparsedPublicKey::new(&ED25519, key).verify(json, &sig).map_err(|_| "güncelleme imzası geçersiz")?;
    let m: Manifest = serde_json::from_slice(json).map_err(|e| format!("manifest okunamadı: {e}"))?;
    m.check(expected)?;
    Ok(m)
}

fn sha256(data: &[u8]) -> String {
    update::to_hex(ring::digest::digest(&ring::digest::SHA256, data).as_ref())
}

fn file_matches(path: &Path, m: &Manifest) -> bool {
    std::fs::read(path).is_ok_and(|d| d.len() as u64 == m.size && sha256(&d) == m.sha256)
}

/// Kurulmus (ya da eskimis) indirmeleri temizler.
fn cleanup_installed(current: &Version) {
    // 3.0.0 oncesi surumlerin indirdigi eski adli paket artik hic kullanilmaz.
    let _ = std::fs::remove_file(shared::paths::update_dir().join(update::LEGACY_INSTALLER_NAME));
    let marker = shared::paths::update_dir().join("version.txt");
    if let Some(v) = std::fs::read_to_string(&marker).ok().and_then(|t| Version::parse(&t)) {
        if v <= *current {
            let _ = std::fs::remove_file(shared::paths::update_installer());
            let _ = std::fs::remove_file(&marker);
        }
    }
}

/// Hazir paketin surumunu not eder (sonraki acilista kurulduysa silinsin diye).
pub fn remember_ready(version: &str) {
    let _ = std::fs::write(shared::paths::update_dir().join("version.txt"), version);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn keypair() -> (Ed25519KeyPair, [u8; 32]) {
        let pk8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pk8.as_ref()).unwrap();
        let pk: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
        (pair, pk)
    }

    fn manifest(version: &str) -> Vec<u8> {
        serde_json::to_vec(&Manifest { version: version.into(), file: update::INSTALLER_NAME.into(), size: 10, sha256: "0".repeat(64) }).unwrap()
    }

    #[test]
    fn accepts_only_correctly_signed_manifests() {
        let (pair, pk) = keypair();
        let v = Version::parse("3.0.1").unwrap();
        let json = manifest("3.0.1");
        let sig = update::to_hex(pair.sign(&json).as_ref());
        assert!(verify_manifest(&json, sig.as_bytes(), &pk, &v).is_ok());

        // Baska anahtarla imzalanmis.
        let (other, _) = keypair();
        let bad = update::to_hex(other.sign(&json).as_ref());
        assert!(verify_manifest(&json, bad.as_bytes(), &pk, &v).is_err());

        // Imzadan sonra degistirilmis manifest.
        let mut tampered = json.clone();
        let i = tampered.iter().position(|&b| b == b'1').unwrap();
        tampered[i] = b'9';
        assert!(verify_manifest(&tampered, sig.as_bytes(), &pk, &v).is_err());

        // Gecerli imzali ama baska surume ait (eski paketi yeni diye sunma).
        let old = manifest("3.0.0");
        let old_sig = update::to_hex(pair.sign(&old).as_ref());
        assert!(verify_manifest(&old, old_sig.as_bytes(), &pk, &v).is_err());

        assert!(verify_manifest(&json, b"bozuk", &pk, &v).is_err());
    }

    #[test]
    fn embedded_key_is_set() {
        assert_ne!(update::PUBLIC_KEY, [0u8; 32], "surum imza anahtari gomulu olmali");
    }
}
