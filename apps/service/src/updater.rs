//! Guncelleme: GitHub surumlerine bakar, yeni surumun kurulum paketini indirir, imzasini
//! dogrular ve "Otomatik yukle" aciksa kendisi sessizce kurar. Servis SYSTEM olarak calistigi
//! icin kullanici hicbir sey yapmaz: Windows onay (UAC) sormaz, ekranda kurulum penceresi
//! acilmaz. Acik arayuzler kurulumdan once kendiliginden kapanir; kurulumdan sonra baslayan
//! yeni servis onlari kullanicinin oturumunda (kullanicinin kendi yetkisiyle) yeniden acar.
//!
//! Dogrulama zinciri: update.json'un Ed25519 imzasi gomulu acik anahtarla dogrulanir ->
//! manifestteki surum yayin etiketiyle ayni ve simdiki surumden yeni olmali -> indirilen
//! paketin boyutu ve SHA-256'si manifestle birebir eslesmeli. Biri tutmazsa dosya silinir.
//! Paket baslatilmadan hemen once ozet yeniden hesaplanir.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use shared::config::{UpdateChannel, UpdateMode};
use shared::update::{self, Candidate, Manifest, Release, UpdateState, Version};

use crate::controller::{Controller, now_ms};
use crate::{https, info, launch, warn};

/// Acilistan sonra ilk denetimden once beklenir (ag baglantisi otursun).
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL_MS: u64 = 6 * 3600 * 1000;
/// Denetim basarisizsa (acilista ag henuz yoksa) 6 saat beklenmez; giderek seyrelen aralikla denenir.
const RETRY_MS: [u64; 5] = [2 * 60_000, 5 * 60_000, 15 * 60_000, 30 * 60_000, 60 * 60_000];
/// Zamanlayicinin uyanma araligi. Sure duvar saatiyle olculur: uyku ya da hazirda bekletmeden
/// donunce gecikmis denetim hemen yapilir.
const TICK: Duration = Duration::from_secs(60);
/// Arayuz acilinca istenen denetim bundan yakin zamanda yapildiysa aga cikilmaz.
const MIN_RECHECK_MS: u64 = 10 * 60 * 1000;
/// Kurulumdan once arayuzlerin kapanmasi icin en fazla bu kadar beklenir (kalani kurulum kapatir).
const UI_CLOSE_WAIT: Duration = Duration::from_secs(5);
/// Kurulum bu surede servisi durdurmazsa basarisiz sayilir.
const INSTALL_TIMEOUT_MS: u32 = 15 * 60 * 1000;

pub struct Updater {
    busy: AtomicBool,
    installing: AtomicBool,
}

impl Updater {
    pub fn new() -> Self {
        Self { busy: AtomicBool::new(false), installing: AtomicBool::new(false) }
    }
}

/// Bir oturumda acik olan arayuz surecleri (kurulumdan sonra yeniden acilacaklar).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSession {
    pub session: u32,
    pub tray: bool,
    pub window: bool,
}

/// Kurulumdan once yazilan not (`paths::update_relaunch_file`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Relaunch {
    version: String,
    sessions: Vec<UiSession>,
}

/// Arka planda duzenli denetim: servis acilisi, sonra her 6 saat; basarisizsa daha sik.
pub fn spawn_schedule(ctrl: &Arc<Controller>) {
    let weak = Arc::downgrade(ctrl);
    std::thread::Builder::new()
        .name("guncelleme".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            let mut failures = 0usize;
            let mut due = 0u64;
            loop {
                let Some(ctrl) = weak.upgrade() else { return };
                if now_ms() >= due {
                    run_check(&ctrl, false);
                    let failed = matches!(ctrl.update_status().state, UpdateState::Failed { .. });
                    failures = if failed { failures + 1 } else { 0 };
                    due = now_ms() + next_check_in(failures);
                }
                drop(ctrl);
                std::thread::sleep(TICK);
            }
        })
        .ok();
}

/// Ust uste `failures` basarisiz denetimden sonra bir sonrakine kadar beklenecek sure (ms).
fn next_check_in(failures: usize) -> u64 {
    match failures {
        0 => CHECK_INTERVAL_MS,
        n => RETRY_MS[(n - 1).min(RETRY_MS.len() - 1)],
    }
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
    // Kurulum surerken denetim "kuruluyor" durumunu ezmesin.
    if updater.installing.load(Ordering::SeqCst) || updater.busy.swap(true, Ordering::SeqCst) {
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
    let ready = matches!(state, UpdateState::Ready { .. });
    ctrl.set_update(state, true);
    updater.busy.store(false, Ordering::SeqCst);
    if ready && config.updates == UpdateMode::Auto {
        let _ = install(ctrl, false);
    }
}

/// Arayuzden gelen "simdi yukle": kurulacak paket yoksa hemen hata doner; varsa kurulum arka
/// planda baslar.
pub fn request_install(ctrl: &Arc<Controller>) -> Result<(), String> {
    if !matches!(ctrl.update_status().state, UpdateState::Ready { .. }) {
        return Err("Kurulacak bir güncelleme yok.".into());
    }
    let weak: Weak<Controller> = Arc::downgrade(ctrl);
    std::thread::Builder::new()
        .name("guncelleme-kur".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            if let Some(ctrl) = weak.upgrade() {
                let _ = install(&ctrl, true);
            }
        })
        .map(drop)
        .map_err(|e| format!("Kurulum başlatılamadı: {e}"))
}

/// Dogrulanmis paketi sessizce kurar. `by_user` degilse daha once kurulamamis surum yeniden
/// denenmez (kurulum dongusu olmasin); kullanici "Simdi yukle" ile yine deneyebilir.
/// Basariliysa donmez: kurulum servisi durdurur ve surec onunla biter.
pub fn install(ctrl: &Arc<Controller>, by_user: bool) -> Result<(), String> {
    let ready = ctrl.update_status().state;
    let UpdateState::Ready { version, sha256: expected, .. } = &ready else {
        return Err("Kurulacak bir güncelleme yok.".into());
    };
    if !ctrl.is_service() {
        return Err("Güncelleme yalnızca motor hizmet olarak çalışırken kurulur.".into());
    }
    if !by_user && failed_version().as_deref() == Some(version.as_str()) {
        return Ok(());
    }
    let updater = ctrl.updater();
    if updater.installing.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let installer = shared::paths::update_installer();
    if !std::fs::read(&installer).is_ok_and(|d| sha256(&d).eq_ignore_ascii_case(expected)) {
        let _ = std::fs::remove_file(&installer);
        warn!("Güncelleme paketi doğrulanamadı; silindi.");
        ctrl.set_update(UpdateState::Failed { message: "Güncelleme paketi doğrulanamadı; yeniden indirilecek.".into() }, false);
        updater.installing.store(false, Ordering::SeqCst);
        return Err("paket doğrulanamadı".into());
    }

    let note = Relaunch { version: version.clone(), sessions: ctrl.ui_sessions() };
    let _ = std::fs::write(shared::paths::update_relaunch_file(), serde_json::to_vec(&note).unwrap_or_default());
    info!("Sürüm {version} kuruluyor; açık arayüzler kapanıp sonra yeniden açılacak ({:?}).", note.sessions);
    ctrl.set_update(UpdateState::Installing { version: version.clone() }, false);

    // Arayuzler Installing durumunu gorunce kendiliginden kapanir (tepsi simgesi temiz kalkar).
    let start = Instant::now();
    while !ctrl.ui_sessions().is_empty() && start.elapsed() < UI_CLOSE_WAIT {
        std::thread::sleep(Duration::from_millis(100));
    }

    let log = shared::paths::log_dir().join("update-setup.log");
    let failure = match launch::run_installer(&installer, &log) {
        // Kurulum servisi durdurur ve bu surec onunla biter. Buraya donulduyse kurulum servise
        // dokunmadan bitti ya da takildi: kurulmamis sayilir.
        Ok(process) => {
            if process.wait(INSTALL_TIMEOUT_MS) {
                "kurulum hizmeti durdurmadan bitti".to_owned()
            } else {
                "kurulum zaman aşımına uğradı".to_owned()
            }
        }
        Err(e) => e,
    };
    warn!("Sürüm {version} kurulamadı: {failure}. Kurulum günlüğü: {}", log.display());
    remember_failed(version);
    let _ = std::fs::remove_file(shared::paths::update_relaunch_file());
    relaunch(&ctrl.exe_dir().join(shared::UI_EXE), &note.sessions);
    ctrl.set_update(ready.clone(), false);
    updater.installing.store(false, Ordering::SeqCst);
    Err(failure)
}

/// Servis acilisi: kurulumdan once yazilan not varsa arayuzleri yeniden acar ve kurulumun
/// tutup tutmadigini kaydeder (tutmadiysa o surum otomatik olarak yeniden denenmez).
pub fn after_start(ctrl: &Arc<Controller>) {
    let path = shared::paths::update_relaunch_file();
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let _ = std::fs::remove_file(&path);
    let Ok(note) = serde_json::from_str::<Relaunch>(&text) else { return };
    if installed(&note.version, shared::VERSION) {
        info!("Sürüm {} kuruldu.", note.version);
        clear_failed();
    } else {
        warn!(
            "Sürüm {} kurulamadı; kurulum günlüğü: {}",
            note.version,
            shared::paths::log_dir().join("update-setup.log").display()
        );
        remember_failed(&note.version);
    }
    let exe = ctrl.exe_dir().join(shared::UI_EXE);
    std::thread::Builder::new()
        .name("arayuzu-ac".into())
        .stack_size(128 * 1024)
        .spawn(move || relaunch(&exe, &note.sessions))
        .ok();
}

/// Kurulumdan sonra calisan surum, kurulmasi istenen surume ulasti mi?
fn installed(wanted: &str, running: &str) -> bool {
    match (Version::parse(wanted), Version::parse(running)) {
        (Some(w), Some(r)) => r >= w,
        _ => false,
    }
}

/// Arayuzu kayitli oturumlarda yeniden acar. Tepsi ve pencere tek ornektir; acik olan yinelenmez.
fn relaunch(exe: &Path, sessions: &[UiSession]) {
    for s in sessions {
        for (wanted, args, what) in [(s.tray, "--tray", "tepsi"), (s.window, "", "pencere")] {
            if !wanted {
                continue;
            }
            match launch::run_as_session_user(s.session, exe, args) {
                Ok(()) => info!("Arayüz yeniden açıldı ({}. oturum, {what}).", s.session),
                Err(e) => warn!("Arayüz yeniden açılamadı ({what}): {e}"),
            }
        }
    }
}

fn failed_file() -> std::path::PathBuf {
    shared::paths::update_dir().join("failed.txt")
}

/// Kurulamayan surum: otomatik kurulum onu bir daha denemez (daha yeni bir surum gelene kadar).
fn remember_failed(version: &str) {
    let _ = std::fs::write(failed_file(), version);
}

fn failed_version() -> Option<String> {
    std::fs::read_to_string(failed_file()).ok().map(|t| t.trim().to_owned())
}

fn clear_failed() {
    let _ = std::fs::remove_file(failed_file());
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

    /// Acilista ag yoksa denetim 6 saat sonraya kalmaz: 2, 5, 15, 30 dk, sonra saatte bir.
    #[test]
    fn failed_checks_retry_soon() {
        assert_eq!(next_check_in(0), 6 * 3600 * 1000);
        let mins: Vec<u64> = (1..=7).map(|n| next_check_in(n) / 60_000).collect();
        assert_eq!(mins, [2, 5, 15, 30, 60, 60, 60]);
    }

    /// Kurulumdan sonra yeni servis, notta yazan surumu calistiriyorsa kurulum tutmustur.
    #[test]
    fn detects_whether_install_took() {
        assert!(installed("3.0.4", "3.0.4"));
        assert!(installed("3.0.4", "3.0.5"));
        assert!(!installed("3.0.4", "3.0.3"), "eski servis geri geldiyse kurulum tutmadi");
        assert!(!installed("bozuk", "3.0.4"));
    }

    #[test]
    fn relaunch_note_round_trips() {
        let note = Relaunch {
            version: "3.0.4".into(),
            sessions: vec![UiSession { session: 1, tray: true, window: true }, UiSession { session: 2, tray: true, window: false }],
        };
        let json = serde_json::to_vec(&note).unwrap();
        assert_eq!(serde_json::from_slice::<Relaunch>(&json).unwrap(), note);
    }
}
