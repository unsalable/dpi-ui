//! v1 (GoodbyeDPI UI 2.x) ayarlarini aktarir: `%AppData%\GoodbyeDPI-UI\settings.json`.
//! Arayuz kullanici olarak calistigi icin dosyayi o okur, sonucu servise gonderir.

use engine::DpiConfig;
use serde::Deserialize;

use crate::config::{AppConfig, CustomDns, CustomDnsKind, CustomMethod, DnsMode};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct V1Settings {
    dark_mode: Option<bool>,
    isp: Option<String>,
    native_profile: Option<String>,
    custom_profiles: Vec<V1Profile>,
    dns: Option<String>,
    custom_dns: Vec<V1Dns>,
    run_at_startup: Option<bool>,
    auto_connect: Option<bool>,
    minimize_to_tray: Option<bool>,
    auto_update: Option<bool>,
    native_custom: Option<DpiConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct V1Profile {
    id: String,
    name: String,
    config: DpiConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct V1Dns {
    id: String,
    name: String,
    v4: String,
    v4_port: u16,
    v6: String,
    v6_port: u16,
}

/// v1'den gelen, arayuzun de kullanacagi tercihler.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct V1UiPrefs {
    pub dark_mode: Option<bool>,
    pub minimize_to_tray: Option<bool>,
}

#[derive(Debug)]
pub struct Migrated {
    pub config: AppConfig,
    pub ui: V1UiPrefs,
    /// Kullaniciya gosterilecek ozet.
    pub summary: Vec<String>,
}

/// v1 ayar metnini yeni yapilandirmaya cevirir. Bozuk dosyada `None`.
pub fn from_v1(json: &str) -> Option<Migrated> {
    let v1: V1Settings = serde_json::from_str(json).ok()?;
    let mut c = AppConfig::default();
    let mut summary = Vec::new();

    if let Some(isp) = v1.isp.as_deref() {
        if engine::presets::ISPS.iter().any(|i| i.id.eq_ignore_ascii_case(isp)) {
            c.isp = isp.to_ascii_lowercase();
            summary.push(format!("İnternet sağlayıcı: {}", engine::presets::isp(isp).name));
        }
    }

    let mut profiles = v1.custom_profiles;
    if profiles.is_empty() {
        if let Some(cfg) = v1.native_custom {
            profiles.push(V1Profile { id: "custom".into(), name: "Özel".into(), config: cfg });
        }
    }
    // v1 her kurulumda bos bir "Ozel" profil olusturuyordu; varsayilandan farkli olanlari al.
    for p in profiles.into_iter().filter(|p| p.config != DpiConfig::default() || p.name != "Özel") {
        c.custom_methods.push(CustomMethod { id: p.id, name: p.name, config: p.config });
    }
    if !c.custom_methods.is_empty() {
        summary.push(format!("{} özel yöntem", c.custom_methods.len()));
    }

    if let Some(m) = v1.native_profile.as_deref() {
        let known = engine::presets::method(m).is_some() || c.custom_methods.iter().any(|p| p.id.eq_ignore_ascii_case(m));
        if known {
            c.method = m.to_owned();
        }
    }

    for d in v1.custom_dns.into_iter().filter(|d| !d.v4.trim().is_empty() || !d.v6.trim().is_empty()) {
        c.dns.custom.push(CustomDns {
            id: d.id,
            name: d.name,
            kind: CustomDnsKind::Udp,
            v4: d.v4,
            v4_port: d.v4_port,
            v6: d.v6,
            v6_port: d.v6_port,
            ..CustomDns::default()
        });
    }
    if !c.dns.custom.is_empty() {
        summary.push(format!("{} özel DNS", c.dns.custom.len()));
    }

    match v1.dns.as_deref().map(str::to_ascii_lowercase).as_deref() {
        // v1'de Cloudflare duz UDP'ydi; ayni saglayicinin sifreli surumu daha saglam.
        Some("cloudflare") | None => {}
        Some("yandex") => {
            c.dns.mode = DnsMode::Redirect;
            c.dns.provider = "yandex".into();
        }
        Some("off") => c.dns.mode = DnsMode::Off,
        Some(id) if c.dns.custom.iter().any(|d| d.id.eq_ignore_ascii_case(id)) => {
            c.dns.mode = DnsMode::Redirect;
            c.dns.provider = id.to_owned();
        }
        Some(_) => {}
    }

    if let Some(v) = v1.run_at_startup {
        c.start_with_windows = v;
    }
    if let Some(v) = v1.auto_connect {
        c.auto_connect = v;
    }
    if v1.auto_update == Some(false) {
        c.updates = crate::config::UpdateMode::Notify;
    }

    let config = c.sanitized();
    Some(Migrated { config, ui: V1UiPrefs { dark_mode: v1.dark_mode, minimize_to_tray: v1.minimize_to_tray }, summary })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_typical_v1_file() {
        let json = r#"{
          "darkMode": true, "engine": 0, "isp": "superonline", "method": "default",
          "nativeProfile": "custom:ab12cd34",
          "customProfiles": [
            {"id":"custom","name":"Özel","config":{}},
            {"id":"custom:ab12cd34","name":"Benim ayarım","config":{"ttl":3,"autoTtl":false,"fakePayload":1}}
          ],
          "dns": "custom",
          "customDns": [{"id":"custom","name":"Ev DNS","v4":"9.9.9.9","v4Port":9953,"v6":"","v6Port":53}],
          "runAtStartup": false, "autoConnect": true, "minimizeToTray": true, "autoUpdate": true
        }"#;
        let m = from_v1(json).unwrap();
        assert_eq!(m.config.isp, "superonline");
        assert_eq!(m.config.method, "custom:ab12cd34");
        assert_eq!(m.config.custom_methods.len(), 1, "bos varsayilan Ozel profil atlanir");
        assert_eq!(m.config.custom_methods[0].config.ttl, 3);
        assert_eq!(m.config.dns.mode, DnsMode::Redirect);
        assert_eq!(m.config.dns.provider, "custom");
        assert!(!m.config.start_with_windows);
        assert_eq!(m.ui.dark_mode, Some(true));
        assert!(m.config.validate().is_ok());
        assert_eq!(m.summary.len(), 3);
    }

    #[test]
    fn tolerates_garbage() {
        assert!(from_v1("not json").is_none());
        let m = from_v1("{}").unwrap();
        assert_eq!(m.config, AppConfig::default().sanitized());
    }
}
