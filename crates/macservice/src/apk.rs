//! Android surumunun guncellemesi: Windows ve Mac'tekiyle ayni imza zinciri, kendi dosyalari.
//!
//! 1. GitHub yayinlarinda simdikinden yeni ve imzali Android paketi olan surum aranir.
//! 2. `update-android.json` gomulu Ed25519 anahtariyla dogrulanir; surum etiketle ayni olmali.
//! 3. `DPI-UI-Android.apk` indirilir; boyutu ve SHA-256'si manifestle birebir eslesmeli.
//! 4. Kurulumu Android'in paket yukleyicisi yapar (uygulama `INSTALLER`i verir). Android 12+
//!    uygulamanin kendi kurdugu surumu onay sormadan gunceller; digerlerinde sistem bir kez sorar.
//!
//! Istekler uygulama icindeki HTTPS istemcisiyle gider (Android'de curl yok).

use std::path::Path;
use std::sync::OnceLock;

use shared::t;
use shared::update::{self, Release, Version};

use crate::http;
use crate::updates::{Prepared, sha256_file, verify_signed};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub version: Version,
    pub notes: String,
    pub package_url: String,
    pub manifest_url: String,
    pub signature_url: String,
}

/// Indirilmis paketi sistemin paket yukleyicisine verir. `interactive`: kullanici "Simdi yukle"
/// dedi (onay gerekirse pencere hemen acilir; kendiliginden kurulumda bildirim gosterilir).
pub type Installer = fn(&Path, bool) -> Result<(), String>;

/// Android uygulamasi baslarken verir (Java paket yukleyicisi).
pub static INSTALLER: OnceLock<Installer> = OnceLock::new();

/// Simdiki surumden yeni ve imzali Android paketi olan en yuksek surum.
pub fn pick(current: &Version, releases: &[Release], include_prerelease: bool) -> Option<Found> {
    let include_pre = include_prerelease || current.is_prerelease();
    releases
        .iter()
        .filter(|r| !r.draft && (include_pre || !r.prerelease))
        .filter_map(|r| {
            let version = Version::parse(&r.tag_name)?;
            if version <= *current || (version.is_prerelease() && !include_pre) {
                return None;
            }
            let url = |name: &str| r.assets.iter().find(|a| a.name.eq_ignore_ascii_case(name)).map(|a| a.browser_download_url.clone());
            Some(Found {
                version,
                notes: r.body.clone().unwrap_or_default().chars().take(2000).collect(),
                package_url: url(update::ANDROID_PACKAGE_NAME)?,
                manifest_url: url(update::ANDROID_MANIFEST_NAME)?,
                signature_url: url(update::ANDROID_SIGNATURE_NAME)?,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

fn allowed(host: &str) -> bool {
    update::allowed_host(host)
}

/// GitHub'dan yayin listesini ceker.
pub fn fetch() -> Result<Vec<Release>, String> {
    let mut body = Vec::new();
    let opts = http::Options { headers: &[("Accept", "application/vnd.github+json")], ..Default::default() };
    http::get_following(update::RELEASES_API, &opts, &allowed, &mut body)?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

/// API'ye ulasilamazsa (ortak IP'li hatlarda saatlik sinir) sitenin yayin akisindan bakar; imzali
/// Android paketi olan en yeni uygun surum.
pub fn pick_from_feed(current: &Version, include_prerelease: bool) -> Result<Option<Found>, String> {
    let include_pre = include_prerelease || current.is_prerelease();
    let mut feed = Vec::new();
    http::get_following(&format!("https://github.com/{}/releases.atom", update::REPO), &http::Options::default(), &allowed, &mut feed)?;
    let feed = String::from_utf8_lossy(&feed);
    let mut versions: Vec<(Version, String)> = crate::updates::feed_tags(&feed)
        .into_iter()
        .filter_map(|tag| Some((Version::parse(tag)?, tag.to_owned())))
        .filter(|(v, _)| *v > *current && (include_pre || !v.is_prerelease()))
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions.dedup_by(|a, b| a.1 == b.1);
    for (version, tag) in versions {
        let url = |name: &str| format!("https://github.com/{}/releases/download/{tag}/{name}", update::REPO);
        let head = http::Options { head_only: true, ..Default::default() };
        if http::get_following(&url(update::ANDROID_MANIFEST_NAME), &head, &allowed, &mut std::io::sink()).is_ok() {
            return Ok(Some(Found {
                version,
                notes: String::new(),
                package_url: url(update::ANDROID_PACKAGE_NAME),
                manifest_url: url(update::ANDROID_MANIFEST_NAME),
                signature_url: url(update::ANDROID_SIGNATURE_NAME),
            }));
        }
    }
    Ok(None)
}

/// Manifesti dogrular, paketi indirir ve denetler. Paket `dir` icinde durur.
pub fn prepare(found: &Found, dir: &Path) -> Result<Prepared, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (mpath, spath) = (dir.join(update::ANDROID_MANIFEST_NAME), dir.join(update::ANDROID_SIGNATURE_NAME));
    http::download(&found.manifest_url, &mpath, 16 * 1024, &allowed)?;
    http::download(&found.signature_url, &spath, 1024, &allowed)?;
    let json = std::fs::read(&mpath).map_err(|e| e.to_string())?;
    let sig = std::fs::read(&spath).map_err(|e| e.to_string())?;
    let m = verify_signed(&json, &sig, &update::PUBLIC_KEY, &found.version, shared::update::Manifest::check_android)?;
    let apk = dir.join(update::ANDROID_PACKAGE_NAME);
    let matches = |p: &Path| sha256_file(p).is_ok_and(|(sha, size)| sha == m.sha256 && size == m.size);
    if !matches(&apk) {
        http::download(&found.package_url, &apk, m.size, &allowed)?;
        if !matches(&apk) {
            let _ = std::fs::remove_file(&apk);
            return Err(t!("indirilen paket imzalı manifestle eşleşmiyor; silindi", "the downloaded package doesn't match the signed manifest; deleted"));
        }
    }
    Ok(Prepared { version: found.version.clone(), zip: apk, sha256: m.sha256 })
}

pub fn install(p: &Prepared, interactive: bool) -> Result<(), String> {
    let installer = INSTALLER.get().ok_or_else(|| t!("Paket yükleyicisi hazır değil.", "The package installer isn't ready."))?;
    installer(&p.zip, interactive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::update::Asset;

    fn release(tag: &str, assets: &[&str], prerelease: bool) -> Release {
        Release {
            tag_name: tag.into(),
            body: Some("notlar".into()),
            draft: false,
            prerelease,
            assets: assets.iter().map(|n| Asset { name: (*n).into(), browser_download_url: format!("https://github.com/x/{n}"), size: 1 }).collect(),
        }
    }

    #[test]
    fn needs_a_signed_apk() {
        let full = [update::ANDROID_PACKAGE_NAME, update::ANDROID_MANIFEST_NAME, update::ANDROID_SIGNATURE_NAME];
        let current = Version::parse("3.1.3").unwrap();
        let list = vec![
            release("v3.2.0", &[update::INSTALLER_NAME, update::MAC_PACKAGE_NAME], false),
            release("v3.1.4", &full, false),
            release("v3.3.0-beta.1", &full, true),
            release("v3.1.2", &full, false),
        ];
        let found = pick(&current, &list, false).unwrap();
        assert_eq!(found.version.to_string(), "3.1.4", "Android paketi olmayan yayin atlanir");
        assert!(found.package_url.ends_with(update::ANDROID_PACKAGE_NAME));
        assert_eq!(pick(&current, &list, true).unwrap().version.to_string(), "3.3.0-beta.1");
        assert!(pick(&Version::parse("3.1.4").unwrap(), &list, false).is_none());
        let unsigned = vec![release("v3.5.0", &[update::ANDROID_PACKAGE_NAME], false)];
        assert!(pick(&current, &unsigned, false).is_none(), "imzasiz paket kurulmaz");
    }
}
