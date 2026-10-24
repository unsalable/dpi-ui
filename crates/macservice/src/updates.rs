//! Mac surumunun guncellemesi: Windows'takiyle ayni imza zinciri, kendi dosyalari.
//!
//! 1. GitHub yayinlarinda simdikinden yeni ve Mac paketi olan surum aranir.
//! 2. `update-mac.json` gomulu Ed25519 anahtariyla dogrulanir; surum etiketle ayni olmali.
//! 3. `DPI-UI-Mac.zip` indirilir; boyutu ve SHA-256'si manifestle birebir eslesmeli.
//! 4. Kurulum: paket acilir, kucuk bir kabuk betigi uygulama kapanmasini bekleyip yeni
//!    `DPI UI.app`'i eskisinin yerine koyar ve uygulamayi yeniden acar. Yonetici gerekmez
//!    (Uygulamalar klasoru yonetici kullaniciya yazilabilir); yazilamiyorsa disk goruntusu
//!    tarayicida acilir.
//!
//! Yalnizca .dmg'si olan (imzali paketi olmayan) yayinlarda guncelleme elle kurulur.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ring::signature::{ED25519, UnparsedPublicKey};
use shared::t;
use shared::update::{self, Manifest, RELEASES_API, Release, Version};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed {
    pub package_url: String,
    pub manifest_url: String,
    pub signature_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub version: Version,
    pub notes: String,
    /// Elle kurulum icin disk goruntusu.
    pub dmg_url: Option<String>,
    /// Sessiz kurulum icin imzali paket.
    pub signed: Option<Signed>,
}

/// Indirilip dogrulanmis paket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub version: Version,
    pub zip: PathBuf,
    pub sha256: String,
}

/// Simdiki surumden yeni ve Mac paketi (imzali ya da yalnizca .dmg) olan en yuksek surum.
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
            let signed = match (url(update::MAC_PACKAGE_NAME), url(update::MAC_MANIFEST_NAME), url(update::MAC_SIGNATURE_NAME)) {
                (Some(package_url), Some(manifest_url), Some(signature_url)) => Some(Signed { package_url, manifest_url, signature_url }),
                _ => None,
            };
            let dmg_url = url(update::MAC_DMG_NAME);
            if signed.is_none() && dmg_url.is_none() {
                return None;
            }
            Some(Found { version, notes: r.body.clone().unwrap_or_default().chars().take(2000).collect(), dmg_url, signed })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// Imzayi dogrular ve manifesti adaya karsi denetler.
pub fn verify_manifest(json: &[u8], sig_text: &[u8], key: &[u8; 32], expected: &Version) -> Result<Manifest, String> {
    let sig = update::from_hex(&String::from_utf8_lossy(sig_text)).ok_or_else(|| t!("imza dosyası bozuk", "the signature file is corrupt"))?;
    UnparsedPublicKey::new(&ED25519, key).verify(json, &sig).map_err(|_| t!("güncelleme imzası geçersiz", "invalid update signature"))?;
    let m: Manifest = serde_json::from_slice(json).map_err(|e| t!("manifest okunamadı: {e}", "couldn't read the manifest: {e}"))?;
    m.check_mac(expected)?;
    Ok(m)
}

pub fn sha256_file(path: &Path) -> Result<(String, u64), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok((update::to_hex(ring::digest::digest(&ring::digest::SHA256, &data).as_ref()), data.len() as u64))
}

fn host_of(url: &str) -> Option<&str> {
    url.strip_prefix("https://")?.split('/').next()
}

/// macOS'ta her zaman bulunan curl ile indirir; yalnizca GitHub adresleri, yalnizca HTTPS.
fn download(url: &str, dest: &Path, max: u64) -> Result<(), String> {
    if !host_of(url).is_some_and(update::allowed_host) {
        return Err(t!("beklenmeyen indirme adresi", "unexpected download address"));
    }
    let out = Command::new("/usr/bin/curl")
        .args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--max-time", "600", "--max-filesize"])
        .arg(max.to_string())
        .arg("-o")
        .arg(dest)
        .arg(url)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let _ = std::fs::remove_file(dest);
        return Err(t!("indirilemedi: {}", "couldn't download: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// GitHub'dan yayin listesini ceker.
pub fn fetch() -> Result<Vec<Release>, String> {
    let out = Command::new("/usr/bin/curl")
        .args(["-fsSL", "--proto", "=https", "--max-time", "20", "-H", "Accept: application/vnd.github+json", "-A", "DPI-UI-Mac", RELEASES_API])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

/// Manifesti dogrular, paketi indirir ve denetler. Paket `dir` icinde durur.
pub fn prepare(found: &Found, signed: &Signed, dir: &Path) -> Result<Prepared, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (mpath, spath) = (dir.join(update::MAC_MANIFEST_NAME), dir.join(update::MAC_SIGNATURE_NAME));
    download(&signed.manifest_url, &mpath, 16 * 1024)?;
    download(&signed.signature_url, &spath, 1024)?;
    let json = std::fs::read(&mpath).map_err(|e| e.to_string())?;
    let sig = std::fs::read(&spath).map_err(|e| e.to_string())?;
    let m = verify_manifest(&json, &sig, &update::PUBLIC_KEY, &found.version)?;
    let zip = dir.join(update::MAC_PACKAGE_NAME);
    let matches = |p: &Path| sha256_file(p).is_ok_and(|(sha, size)| sha == m.sha256 && size == m.size);
    if !matches(&zip) {
        download(&signed.package_url, &zip, m.size)?;
        if !matches(&zip) {
            let _ = std::fs::remove_file(&zip);
            return Err(t!("indirilen paket imzalı manifestle eşleşmiyor; silindi", "the downloaded package doesn't match the signed manifest; deleted"));
        }
    }
    Ok(Prepared { version: found.version.clone(), zip, sha256: m.sha256 })
}

/// Calisan uygulamanin paketi (`.../DPI UI.app`).
pub fn bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

/// Uygulama bu konumda kendini guncelleyebilir mi? Disk goruntusunden calisiyorsa ya da klasor
/// yazilamiyorsa (standart kullanici) hayir.
pub fn can_self_update(bundle: &Path) -> Result<(), String> {
    if bundle.starts_with("/Volumes") {
        return Err(t!(
            "Uygulama disk görüntüsünden çalışıyor; önce Uygulamalar klasörüne sürükle.",
            "The app is running from the disk image; drag it to Applications first."
        ));
    }
    let parent = bundle.parent().ok_or("paket klasörü yok")?;
    let probe = parent.join(".dpi-ui-yazma-denemesi");
    match std::fs::OpenOptions::new().create(true).truncate(true).write(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(_) => Err(t!(
            "{} klasörüne yazılamıyor; güncelleme elle kurulmalı.",
            "Can't write to {}; the update has to be installed manually.",
            parent.display()
        )),
    }
}

/// Kurulum betigi: uygulama (PID) kapaninca yeni paketi yerine koyar ve yeniden acar. Kapanmazsa
/// 30 sn sonra kapatir. Yer degistirme tutmazsa eski paket geri konur.
pub const INSTALL_SCRIPT: &str = r#"#!/bin/sh
# DPI UI guncellemesi. Kullanim: install.sh PID YENI_PAKET HEDEF_PAKET [background]
PID="$1"; NEW="$2"; DEST="$3"; MODE="$4"
i=0
while kill -0 "$PID" 2>/dev/null; do
    i=$((i + 1))
    [ "$i" -eq 150 ] && kill "$PID" 2>/dev/null
    [ "$i" -ge 200 ] && kill -9 "$PID" 2>/dev/null
    sleep 0.2
done
OLD="$DEST.eski"
rm -rf "$OLD"
if mv "$DEST" "$OLD"; then
    if mv "$NEW" "$DEST"; then
        rm -rf "$OLD"
    else
        mv "$OLD" "$DEST"
    fi
fi
if [ "$MODE" = "background" ]; then
    /usr/bin/open -g -a "$DEST" --args --background
else
    /usr/bin/open -a "$DEST"
fi
"#;

/// Paketi acar, yeni uygulamayi denetler ve kurulum betigini baslatir. Cagiran surec (PID)
/// bundan sonra kapanmali; betik kapanmasini bekler.
pub fn install(zip: &Path, dir: &Path, bundle: &Path, pid: u32, background: bool) -> Result<(), String> {
    can_self_update(bundle)?;
    let work = dir.join("yeni");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let out = Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(zip).arg(&work).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(t!("paket açılamadı: {}", "couldn't unpack the package: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let new_app = work.join("DPI UI.app");
    let plist = std::fs::read_to_string(new_app.join("Contents/Info.plist")).unwrap_or_default();
    if !new_app.join("Contents/MacOS/DPI UI").is_file() || !plist.contains("com.dpi-ui.app") {
        return Err(t!("paketteki uygulama beklenen biçimde değil", "the app in the package isn't in the expected form"));
    }
    let script = dir.join("install.sh");
    std::fs::write(&script, INSTALL_SCRIPT).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("/bin/sh");
    cmd.arg(&script).arg(pid.to_string()).arg(&new_app).arg(bundle);
    if background {
        cmd.arg("background");
    }
    // Betik uygulamadan bagimsiz yasar (uygulama kapaninca launchd'ye gecer).
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn release(tag: &str, assets: &[&str], pre: bool) -> Release {
        let json = format!(
            r#"{{"tag_name":"{tag}","prerelease":{pre},"assets":[{}]}}"#,
            assets.iter().map(|a| format!(r#"{{"name":"{a}","browser_download_url":"https://github.com/x/{tag}/{a}","size":1}}"#)).collect::<Vec<_>>().join(",")
        );
        serde_json::from_str(&json).unwrap()
    }

    const SIGNED: [&str; 3] = [update::MAC_PACKAGE_NAME, update::MAC_MANIFEST_NAME, update::MAC_SIGNATURE_NAME];

    #[test]
    fn prefers_signed_packages_and_falls_back_to_the_disk_image() {
        let cur = Version::parse("3.2.0").unwrap();
        let mut signed_assets = SIGNED.to_vec();
        signed_assets.push(update::MAC_DMG_NAME);
        let list = vec![
            release("v3.2.2", &["DPI-UI-Setup.exe"], false),
            release("v3.2.1", &signed_assets, false),
            release("v3.3.0-beta.1", &SIGNED, true),
            release("v3.1.0", &SIGNED, false),
        ];
        let f = pick(&cur, &list, false).unwrap();
        assert_eq!(f.version.to_string(), "3.2.1");
        assert!(f.signed.as_ref().unwrap().package_url.ends_with("/v3.2.1/DPI-UI-Mac.zip"));
        assert!(f.dmg_url.unwrap().ends_with("/v3.2.1/DPI-UI-Mac.dmg"));
        assert_eq!(pick(&cur, &list, true).unwrap().version.to_string(), "3.3.0-beta.1");
        assert!(pick(&Version::parse("3.2.1").unwrap(), &list, false).is_none());

        // Yalnizca .dmg: elle kurulum.
        let dmg_only = vec![release("v3.2.5", &[update::MAC_DMG_NAME], false)];
        let f = pick(&cur, &dmg_only, false).unwrap();
        assert!(f.signed.is_none() && f.dmg_url.is_some());
    }

    #[test]
    fn manifest_must_be_signed_and_match() {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
        let v = Version::parse("3.2.0").unwrap();
        let m = |file: &str, version: &str| {
            serde_json::to_vec(&Manifest { version: version.into(), file: file.into(), size: 10, sha256: "a".repeat(64) }).unwrap()
        };
        let sign = |json: &[u8]| update::to_hex(pair.sign(json).as_ref());

        let good = m(update::MAC_PACKAGE_NAME, "3.2.0");
        assert!(verify_manifest(&good, sign(&good).as_bytes(), &key, &v).is_ok());
        assert!(verify_manifest(&good, sign(&good).as_bytes(), &update::PUBLIC_KEY, &v).is_err(), "baska anahtar");
        let mut tampered = good.clone();
        *tampered.last_mut().unwrap() = b' ';
        assert!(verify_manifest(&tampered, sign(&good).as_bytes(), &key, &v).is_err(), "degistirilmis manifest");
        let old = m(update::MAC_PACKAGE_NAME, "3.1.0");
        assert!(verify_manifest(&old, sign(&old).as_bytes(), &key, &v).is_err(), "eski surum yeni diye sunulamaz");
        let windows = m(update::INSTALLER_NAME, "3.2.0");
        assert!(verify_manifest(&windows, sign(&windows).as_bytes(), &key, &v).is_err(), "Windows paketi Mac'e kurulmaz");
    }

    #[test]
    fn only_github_downloads() {
        assert!(download("https://evil.example/x.zip", Path::new("x"), 1).is_err());
        assert!(download("http://github.com/x.zip", Path::new("x"), 1).is_err());
        assert_eq!(host_of("https://github.com/a/b"), Some("github.com"));
    }
}
