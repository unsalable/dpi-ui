//! Guncelleme mantigi (ag ve imza dogrulamasi disindaki her sey): surum karsilastirma,
//! GitHub surum listesinden aday secme, imzali manifest bicimi.
//!
//! Her surumde uc dosya bulunur:
//!   GoodbyeDPI-UI-3-Setup.exe   kurulum paketi
//!   update.json                 {"version", "file", "size", "sha256"}
//!   update.json.sig             update.json'un Ed25519 imzasi (onaltilik metin)
//! Manifest imzalidir ve surumu da icerir: eski bir paket "yeni surum" diye sunulamaz,
//! kurulum dosyasi da manifestteki SHA-256 ile birebir eslesmek zorundadir.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

pub const REPO: &str = "unsalable/dpi-ui";
pub const RELEASES_API: &str = "https://api.github.com/repos/unsalable/dpi-ui/releases?per_page=20";
pub const RELEASES_PAGE: &str = "https://github.com/unsalable/dpi-ui/releases";
pub const SOURCE_PAGE: &str = "https://github.com/unsalable/dpi-ui";
pub const INSTALLER_NAME: &str = "GoodbyeDPI-UI-3-Setup.exe";
pub const MANIFEST_NAME: &str = "update.json";
pub const SIGNATURE_NAME: &str = "update.json.sig";

/// Surum imzalarini dogrulayan Ed25519 acik anahtari. Ozel anahtar depoda degildir;
/// yalnizca surum yayinlayan bilgisayarda durur (`tools/release`).
pub const PUBLIC_KEY: [u8; 32] = [
    0xed, 0x2b, 0xaf, 0x41, 0x14, 0x34, 0xb7, 0xc8, 0x9c, 0xce, 0x96, 0xa3, 0xcf, 0x3d, 0x9b, 0x1a, 0x29, 0x42, 0xe8, 0xef, 0x55, 0x12,
    0xd3, 0x14, 0x47, 0x0b, 0xea, 0xb3, 0x4e, 0x50, 0x1b, 0xfb,
];

// ---------------------------------------------------------------- surum

/// `ana.ara.yama[-on]` (or. 3.0.0-alpha.2). On surum, ayni numarali kararlidan kucuktur.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, p.split('.').map(str::to_owned).collect()),
            None => (s, Vec::new()),
        };
        let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
        let v = Version { major: it.next()??, minor: it.next()??, patch: it.next()??, pre };
        if it.next().is_some() || v.pre.iter().any(|p| p.is_empty()) {
            return None;
        }
        Some(v)
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- GitHub surumleri

#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

/// Kurulabilir bir guncelleme: indirilecek uc dosyanin adresi.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub version: Version,
    pub installer_url: String,
    pub manifest_url: String,
    pub signature_url: String,
    pub notes: String,
}

/// Simdiki surumden yeni ve uc dosyasi da olan en yuksek surumu secer. Simdiki surum on
/// surumse (alpha/beta) ya da kanal Beta ise on surumler de sayilir.
pub fn pick(current: &Version, releases: &[Release], include_prerelease: bool) -> Option<Candidate> {
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
            Some(Candidate {
                installer_url: url(INSTALLER_NAME)?,
                manifest_url: url(MANIFEST_NAME)?,
                signature_url: url(SIGNATURE_NAME)?,
                notes: r.body.clone().unwrap_or_default(),
                version,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// Indirme adresleri yalnizca GitHub'a gidebilir (yonlendirmeler dahil).
pub fn allowed_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "api.github.com"
        || h == "github.com"
        || h == "objects.githubusercontent.com"
        || h == "release-assets.githubusercontent.com"
        || h.ends_with(".githubusercontent.com")
}

// ---------------------------------------------------------------- manifest

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub file: String,
    pub size: u64,
    /// Kurulum dosyasinin SHA-256 ozeti (kucuk harf onaltilik).
    pub sha256: String,
}

impl Manifest {
    /// Imzasi dogrulanmis manifesti adaya karsi denetler.
    pub fn check(&self, expected: &Version) -> Result<(), String> {
        let v = Version::parse(&self.version).ok_or("manifestteki sürüm okunamadı")?;
        if v != *expected {
            return Err(format!("manifest sürümü ({v}) yayın etiketiyle ({expected}) eşleşmiyor"));
        }
        if !self.file.eq_ignore_ascii_case(INSTALLER_NAME) {
            return Err("manifest beklenmeyen bir dosyayı gösteriyor".into());
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("manifestteki SHA-256 geçersiz".into());
        }
        if self.size == 0 || self.size > MAX_INSTALLER {
            return Err("manifestteki boyut geçersiz".into());
        }
        Ok(())
    }
}

/// Kurulum paketinin en buyuk boyutu.
pub const MAX_INSTALLER: u64 = 128 * 1024 * 1024;

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

// ---------------------------------------------------------------- durum (IPC)

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateState {
    /// Henuz denetlenmedi ya da guncelleme kapali.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Downloading { version: String, percent: u8 },
    /// Imzasi dogrulandi, `paths::update_installer()` konumunda kuruluma hazir. Arayuz paketi
    /// baslatmadan once SHA-256'yi yeniden hesaplayip bununla karsilastirir.
    Ready { version: String, notes: String, sha256: String },
    Failed { message: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateStatus {
    #[serde(flatten)]
    pub state: UpdateState,
    /// Son basarili denetim (Unix ms).
    pub checked_at_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_order_like_semver() {
        let order = ["3.0.0-alpha.1", "3.0.0-alpha.2", "3.0.0-alpha.10", "3.0.0-beta", "3.0.0-beta.2", "3.0.0-rc.1", "3.0.0", "3.0.1", "3.1.0", "4.0.0"];
        for w in order.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
        }
        assert_eq!(v("v3.0.0"), v("3.0.0"));
        assert_eq!(v("3.0.0-alpha.2").to_string(), "3.0.0-alpha.2");
        for bad in ["", "3", "3.0", "3.0.x", "3.0.0.1", "3.0.0-", "3.0.0-a..b"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
    }

    fn release(tag: &str, pre: bool, files: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: pre,
            body: Some(format!("{tag} notları")),
            assets: files
                .iter()
                .map(|n| Asset { name: (*n).into(), browser_download_url: format!("https://github.com/{REPO}/releases/download/{tag}/{n}"), size: 1 })
                .collect(),
        }
    }

    const ALL: &[&str] = &[INSTALLER_NAME, MANIFEST_NAME, SIGNATURE_NAME];

    #[test]
    fn picks_newest_complete_release() {
        let rels = vec![
            release("v3.0.0-alpha.1", true, ALL),
            release("v3.0.0-alpha.3", true, &[INSTALLER_NAME]), // imzasiz: atlanir
            release("v3.0.0-alpha.2", true, ALL),
            Release { draft: true, ..release("v3.0.0-alpha.9", true, ALL) },
        ];
        let c = pick(&v("3.0.0-alpha.1"), &rels, false).unwrap();
        assert_eq!(c.version, v("3.0.0-alpha.2"));
        assert!(c.manifest_url.ends_with("/v3.0.0-alpha.2/update.json"));
        assert!(pick(&v("3.0.0-alpha.2"), &rels, false).is_none(), "ayni surum guncelleme degil");
    }

    #[test]
    fn stable_users_skip_prereleases() {
        let rels = vec![release("v3.1.0-beta.1", true, ALL), release("v3.0.1", false, ALL)];
        assert_eq!(pick(&v("3.0.0"), &rels, false).unwrap().version, v("3.0.1"));
        assert_eq!(pick(&v("3.0.0"), &rels, true).unwrap().version, v("3.1.0-beta.1"));
        // On surum kullanicisi kararli surume de gecer.
        assert_eq!(pick(&v("3.0.1-rc.1"), &rels, false).unwrap().version, v("3.1.0-beta.1"));
    }

    #[test]
    fn manifest_must_match_release() {
        let m = Manifest { version: "3.0.0-alpha.2".into(), file: INSTALLER_NAME.into(), size: 7_000_000, sha256: "a".repeat(64) };
        assert!(m.check(&v("3.0.0-alpha.2")).is_ok());
        assert!(m.check(&v("3.0.0-alpha.3")).is_err(), "eski paketi yeni diye sunma");
        assert!(Manifest { file: "baska.exe".into(), ..m.clone() }.check(&v("3.0.0-alpha.2")).is_err());
        assert!(Manifest { sha256: "zz".into(), ..m.clone() }.check(&v("3.0.0-alpha.2")).is_err());
        assert!(Manifest { size: 0, ..m }.check(&v("3.0.0-alpha.2")).is_err());
    }

    #[test]
    fn only_github_hosts() {
        assert!(allowed_host("api.github.com"));
        assert!(allowed_host("release-assets.githubusercontent.com"));
        assert!(!allowed_host("github.com.evil.example"));
        assert!(!allowed_host("example.com"));
    }

    #[test]
    fn hex_roundtrip() {
        let b = [0u8, 1, 0xab, 0xff];
        assert_eq!(from_hex(&to_hex(&b)).unwrap(), b);
        assert!(from_hex("abc").is_none());
        assert!(from_hex("zz").is_none());
    }

    #[test]
    fn status_json_is_stable() {
        let s = UpdateStatus { state: UpdateState::Ready { version: "3.0.1".into(), notes: String::new(), sha256: "ab".into() }, checked_at_ms: Some(1) };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"state\":\"ready\""), "{j}");
        assert_eq!(serde_json::from_str::<UpdateStatus>(&j).unwrap(), s);
    }
}
