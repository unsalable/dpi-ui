//! Mac surumunun guncelleme denetimi: GitHub yayinlarinda Mac paketi (`DPI-UI-Mac.dmg`) olan
//! daha yeni bir surum var mi? Kurulum su an tarayicidan indirilip elle yapilir; sessiz kurulum
//! (Windows'taki gibi imzali paket) sonraki adim.

use shared::update::{RELEASES_API, Release, Version};

pub const MAC_ASSET: &str = "DPI-UI-Mac.dmg";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub version: Version,
    pub url: String,
    pub notes: String,
}

/// Simdiki surumden yeni ve Mac paketi olan en yuksek surum.
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
            let asset = r.assets.iter().find(|a| a.name.eq_ignore_ascii_case(MAC_ASSET))?;
            Some(Found { version, url: asset.browser_download_url.clone(), notes: r.body.clone().unwrap_or_default() })
        })
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// GitHub'dan yayin listesini ceker (macOS'ta her zaman bulunan curl ile).
pub fn fetch() -> Result<Vec<Release>, String> {
    let out = std::process::Command::new("/usr/bin/curl")
        .args(["-fsSL", "--max-time", "20", "-H", "Accept: application/vnd.github+json", "-A", "DPI-UI-Mac", RELEASES_API])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str], pre: bool) -> Release {
        let json = format!(
            r#"{{"tag_name":"{tag}","prerelease":{pre},"assets":[{}]}}"#,
            assets.iter().map(|a| format!(r#"{{"name":"{a}","browser_download_url":"https://github.com/x/{tag}/{a}","size":1}}"#)).collect::<Vec<_>>().join(",")
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn picks_only_releases_with_a_mac_package() {
        let cur = Version::parse("3.2.0").unwrap();
        let list = vec![
            release("v3.2.2", &["DPI-UI-Setup.exe"], false),
            release("v3.2.1", &["DPI-UI-Setup.exe", MAC_ASSET], false),
            release("v3.3.0-beta.1", &[MAC_ASSET], true),
            release("v3.1.0", &[MAC_ASSET], false),
        ];
        let f = pick(&cur, &list, false).unwrap();
        assert_eq!(f.version.to_string(), "3.2.1");
        assert!(f.url.ends_with("/v3.2.1/DPI-UI-Mac.dmg"));
        assert_eq!(pick(&cur, &list, true).unwrap().version.to_string(), "3.3.0-beta.1");
        assert!(pick(&Version::parse("3.2.1").unwrap(), &list, false).is_none());
    }
}
