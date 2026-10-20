//! Servisin sakladigi uygulama ayarlari (`%ProgramData%\GoodbyeDPI3\config.json`).
//! Tek dogruluk kaynagi servistir; arayuz degisiklikleri IPC ile gonderir ve
//! servis dogrulayip kaydeder. Arayuze ozel tercihler (tema, dil) ayri tutulur.

use std::net::{Ipv4Addr, Ipv6Addr};

use engine::{DpiConfig, ScopeMode, presets};
use serde::{Deserialize, Serialize};

use crate::dns;

pub const CONFIG_VERSION: u32 = 1;

const MAX_CUSTOM_METHODS: usize = 64;
const MAX_CUSTOM_DNS: usize = 32;
const MAX_HOSTS: usize = 4000;
const MAX_NAME: usize = 48;
const MAX_PROCESSES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub version: u32,
    /// Secili internet saglayicisi ([`presets::ISPS`] kimligi).
    pub isp: String,
    /// Secili yontem: hazir yontem kimligi ya da ozel yontemin kimligi.
    pub method: String,
    pub custom_methods: Vec<CustomMethod>,
    pub dns: DnsSettings,
    pub scope: ScopeSettings,
    /// Servis acilinca (ya da arayuz acilinca) baglantiyi da baslat.
    pub auto_connect: bool,
    /// Servis Windows ile otomatik baslasin (aksi halde el ile).
    pub start_with_windows: bool,
    pub updates: UpdateMode,
    pub update_channel: UpdateChannel,
    /// Bu surecler calisirken motor duraklatilir (anti-cheat uyumu).
    pub pause_for: Vec<String>,
    pub verbose_log: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            isp: "general".into(),
            method: presets::DEFAULT.id.into(),
            custom_methods: Vec::new(),
            dns: DnsSettings::default(),
            scope: ScopeSettings::default(),
            auto_connect: true,
            start_with_windows: true,
            updates: UpdateMode::Auto,
            update_channel: UpdateChannel::Stable,
            pause_for: Vec::new(),
            verbose_log: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomMethod {
    pub id: String,
    pub name: String,
    pub config: DpiConfig,
}

impl Default for CustomMethod {
    fn default() -> Self {
        Self { id: "custom".into(), name: "Özel".into(), config: DpiConfig::default() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DnsMode {
    /// Sistem DNS'ine dokunma.
    Off,
    /// Sifreli DNS (DoH): sorgular yakalanir, servis cozer.
    #[default]
    Encrypted,
    /// Duz UDP sorgulari secilen sunucunun portuna yonlendirilir (v1 yontemi).
    Redirect,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DnsSettings {
    pub mode: DnsMode,
    /// Hazir saglayici kimligi ya da ozel DNS kimligi.
    pub provider: String,
    pub custom: Vec<CustomDns>,
    /// Sifreli DNS ulasilamazsa gecici olarak sistem DNS'ine dus.
    pub fallback_to_system: bool,
    /// VPN bagliyken DNS'e dokunma.
    pub skip_on_vpn: bool,
}

impl Default for DnsSettings {
    fn default() -> Self {
        Self {
            mode: DnsMode::Encrypted,
            provider: "cloudflare".into(),
            custom: Vec::new(),
            fallback_to_system: true,
            skip_on_vpn: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CustomDnsKind {
    /// Duz UDP: IPv4/IPv6 adres + port.
    #[default]
    Udp,
    /// DNS over HTTPS: URL + baslangic IP'leri.
    Doh,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomDns {
    pub id: String,
    pub name: String,
    pub kind: CustomDnsKind,
    pub v4: String,
    pub v4_port: u16,
    pub v6: String,
    pub v6_port: u16,
    pub doh_url: String,
    /// Virgulle ayrilmis baslangic IP'leri (DoH sunucusunun adresi).
    pub bootstrap: String,
}

impl CustomDns {
    pub fn validate(&self) -> Result<(), String> {
        match self.kind {
            CustomDnsKind::Udp => {
                let v4 = self.v4.trim();
                let v6 = self.v6.trim();
                if v4.is_empty() && v6.is_empty() {
                    return Err("En az bir IPv4 ya da IPv6 adresi gir.".into());
                }
                if !v4.is_empty() && v4.parse::<Ipv4Addr>().is_err() {
                    return Err(format!("“{v4}” geçerli bir IPv4 adresi değil."));
                }
                if !v6.is_empty() && v6.parse::<Ipv6Addr>().is_err() {
                    return Err(format!("“{v6}” geçerli bir IPv6 adresi değil."));
                }
                Ok(())
            }
            CustomDnsKind::Doh => {
                let url = self.doh_url.trim();
                if dns::url_host(url).is_none() {
                    return Err("DoH adresi https:// ile başlamalı (örn. https://dns.ornek.com/dns-query).".into());
                }
                let ips = self.bootstrap_ips();
                if ips.is_empty() {
                    return Err("DoH sunucusunun IP adresini de gir (başlangıç adresi).".into());
                }
                for ip in self.bootstrap.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if ip.parse::<std::net::IpAddr>().is_err() {
                        return Err(format!("“{ip}” geçerli bir IP adresi değil."));
                    }
                }
                Ok(())
            }
        }
    }

    pub fn bootstrap_ips(&self) -> Vec<std::net::IpAddr> {
        self.bootstrap.split(',').filter_map(|s| s.trim().parse().ok()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScopeSettings {
    pub mode: ScopeMode,
    /// Etkin hazir liste kategorileri ([`engine::scope::BUILTIN_TARGETS`]).
    pub categories: Vec<String>,
    /// Kullanicinin ekledigi hedef alan adlari.
    pub targets: Vec<String>,
    /// Asla dokunulmayacak alan adlari.
    pub excluded: Vec<String>,
    /// Akilli modda otomatik ogrenme.
    pub learning: bool,
}

impl Default for ScopeSettings {
    fn default() -> Self {
        Self {
            mode: ScopeMode::Smart,
            categories: engine::scope::BUILTIN_TARGETS.iter().map(|(c, _)| (*c).to_owned()).collect(),
            targets: Vec::new(),
            excluded: Vec::new(),
            learning: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateMode {
    #[default]
    Auto,
    Notify,
    Off,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

/// Secili yontemin cozulmus hali.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveMethod {
    pub id: String,
    pub name: String,
    pub config: DpiConfig,
}

/// Secili DNS'in cozulmus hali.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveDns {
    Off,
    Encrypted { name: String, url: String, bootstrap: Vec<std::net::IpAddr> },
    Redirect { name: String, v4: Option<(Ipv4Addr, u16)>, v6: Option<(Ipv6Addr, u16)> },
}

impl ActiveDns {
    pub fn display_name(&self) -> String {
        match self {
            Self::Off => "Sistem DNS'i".into(),
            Self::Encrypted { name, .. } => format!("{name} DoH"),
            Self::Redirect { name, v4, v6 } => {
                let port = v4.map(|(_, p)| p).or(v6.map(|(_, p)| p)).unwrap_or(53);
                if port == 53 { name.clone() } else { format!("{name} ({port})") }
            }
        }
    }
}

pub fn is_custom_id(id: &str) -> bool {
    id.eq_ignore_ascii_case("custom") || id.to_ascii_lowercase().starts_with("custom:")
}

impl AppConfig {
    /// Degerleri guvenli araliklara ceker; eksik/bozuk alanlari onarir.
    pub fn sanitized(mut self) -> Self {
        self.version = CONFIG_VERSION;
        self.custom_methods.truncate(MAX_CUSTOM_METHODS);
        for m in &mut self.custom_methods {
            m.name = clean_name(&m.name, "Özel");
            m.config = m.config.clone().sanitized();
        }
        dedup_ids(&mut self.custom_methods, |m| &mut m.id);
        self.dns.custom.truncate(MAX_CUSTOM_DNS);
        for d in &mut self.dns.custom {
            d.name = clean_name(&d.name, "Özel DNS");
            if d.v4_port == 0 {
                d.v4_port = 53;
            }
            if d.v6_port == 0 {
                d.v6_port = 53;
            }
        }
        dedup_ids(&mut self.dns.custom, |d| &mut d.id);
        for list in [&mut self.scope.targets, &mut self.scope.excluded] {
            let mut cleaned: Vec<String> = list.iter().filter_map(|h| engine::scope::normalize(h)).collect();
            cleaned.sort();
            cleaned.dedup();
            cleaned.truncate(MAX_HOSTS);
            *list = cleaned;
        }
        self.pause_for.retain(|p| !p.trim().is_empty() && p.len() <= 64);
        self.pause_for.truncate(MAX_PROCESSES);
        if self.resolve_method_opt().is_none() {
            self.method = presets::DEFAULT.id.into();
        }
        if presets::ISPS.iter().all(|i| !i.id.eq_ignore_ascii_case(&self.isp)) {
            self.isp = "general".into();
        }
        self
    }

    /// Arayuzden gelen ayar icin kullaniciya gosterilecek hata.
    pub fn validate(&self) -> Result<(), String> {
        if self.resolve_method_opt().is_none() {
            return Err("Seçili yöntem bulunamadı.".into());
        }
        for d in &self.dns.custom {
            d.validate().map_err(|e| format!("{}: {e}", d.name))?;
        }
        if self.dns.mode != DnsMode::Off && self.resolve_dns_opt().is_none() {
            return Err("Seçili DNS bu modu desteklemiyor ya da bulunamadı.".into());
        }
        Ok(())
    }

    fn resolve_method_opt(&self) -> Option<ActiveMethod> {
        if let Some(m) = presets::method(&self.method) {
            return Some(ActiveMethod { id: m.id.into(), name: m.name.into(), config: m.config() });
        }
        self.custom_methods
            .iter()
            .find(|c| c.id.eq_ignore_ascii_case(&self.method))
            .map(|c| ActiveMethod { id: c.id.clone(), name: c.name.clone(), config: c.config.clone().sanitized() })
    }

    pub fn active_method(&self) -> ActiveMethod {
        self.resolve_method_opt().unwrap_or_else(|| ActiveMethod {
            id: presets::DEFAULT.id.into(),
            name: presets::DEFAULT.name.into(),
            config: presets::DEFAULT.config(),
        })
    }

    fn resolve_dns_opt(&self) -> Option<ActiveDns> {
        if self.dns.mode == DnsMode::Off {
            return Some(ActiveDns::Off);
        }
        if let Some(p) = dns::preset(&self.dns.provider) {
            return match self.dns.mode {
                DnsMode::Encrypted if p.supports_doh() => Some(ActiveDns::Encrypted {
                    name: p.name.into(),
                    url: p.doh_url.into(),
                    bootstrap: p.bootstrap_v4.iter().chain(p.bootstrap_v6).filter_map(|s| s.parse().ok()).collect(),
                }),
                DnsMode::Redirect if p.supports_redirect() => Some(ActiveDns::Redirect {
                    name: p.name.into(),
                    v4: p.udp_v4.and_then(|(a, port)| Some((a.parse().ok()?, port))),
                    v6: p.udp_v6.and_then(|(a, port)| Some((a.parse().ok()?, port))),
                }),
                _ => None,
            };
        }
        let c = self.dns.custom.iter().find(|c| c.id.eq_ignore_ascii_case(&self.dns.provider))?;
        c.validate().ok()?;
        match (self.dns.mode, c.kind) {
            (DnsMode::Encrypted, CustomDnsKind::Doh) => Some(ActiveDns::Encrypted {
                name: c.name.clone(),
                url: c.doh_url.trim().into(),
                bootstrap: c.bootstrap_ips(),
            }),
            (DnsMode::Redirect, CustomDnsKind::Udp) | (DnsMode::Encrypted, CustomDnsKind::Udp) => Some(ActiveDns::Redirect {
                name: c.name.clone(),
                v4: c.v4.trim().parse().ok().map(|a| (a, c.v4_port)),
                v6: c.v6.trim().parse().ok().map(|a| (a, c.v6_port)),
            }),
            _ => None,
        }
    }

    /// Secili DNS; desteklenmiyorsa guvenli taraf: kapali.
    pub fn active_dns(&self) -> ActiveDns {
        self.resolve_dns_opt().unwrap_or(ActiveDns::Off)
    }

    /// Motor kapsamini kurar (ogrenilenler cagiran tarafca eklenir).
    pub fn build_scope(&self, learned: impl IntoIterator<Item = String>) -> engine::Scope {
        let cats: Vec<&str> = self.scope.categories.iter().map(String::as_str).collect();
        let mut targets = engine::HostSet::default();
        if !cats.is_empty() {
            for h in engine::scope::builtin_targets(&cats) {
                targets.insert(h);
            }
        }
        for h in &self.scope.targets {
            targets.insert(h);
        }
        // Sifreli DNS sunucusunun kendisi engelliyse DNS de calismaz: hedeflere ekle.
        if let ActiveDns::Encrypted { url, .. } = self.active_dns() {
            if let Some(h) = dns::url_host(&url) {
                targets.insert(h);
            }
        }
        engine::Scope {
            mode: self.scope.mode,
            targets,
            learned: engine::HostSet::new(learned),
            excluded: engine::HostSet::new(&self.scope.excluded),
            learning: self.scope.learning,
        }
    }

    /// Motoru yeniden baslatmayi gerektiren alanlar degisti mi?
    pub fn engine_differs(&self, other: &AppConfig) -> bool {
        self.active_method() != other.active_method()
            || self.active_dns() != other.active_dns()
            || self.scope != other.scope
            || self.dns.skip_on_vpn != other.dns.skip_on_vpn
    }
}

fn clean_name(name: &str, fallback: &str) -> String {
    let t: String = name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
    if t.is_empty() { fallback.to_owned() } else { t }
}

fn dedup_ids<T>(items: &mut [T], id: impl Fn(&mut T) -> &mut String) {
    let mut seen = std::collections::HashSet::new();
    for (i, item) in items.iter_mut().enumerate() {
        let current = id(item);
        let valid = is_custom_id(current) && current.len() <= 40;
        if !valid || !seen.insert(current.to_ascii_lowercase()) {
            *current = if i == 0 && seen.insert("custom".into()) { "custom".into() } else { format!("custom:{i:04}") };
            seen.insert(current.to_ascii_lowercase());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve() {
        let c = AppConfig::default();
        assert!(c.validate().is_ok());
        assert_eq!(c.active_method().id, "default");
        assert!(matches!(c.active_dns(), ActiveDns::Encrypted { ref url, .. } if url.contains("cloudflare-dns.com")));
        let scope = c.build_scope(vec!["learned.example".to_owned()]);
        assert!(scope.targets.matches("gateway.discord.gg"));
        assert!(scope.targets.matches("cloudflare-dns.com"), "DoH sunucusu hedeflerde");
        assert!(scope.learned.matches("learned.example"));
    }

    #[test]
    fn sanitizes_bad_input() {
        let mut c = AppConfig::default();
        c.method = "yok".into();
        c.isp = "bilinmeyen".into();
        c.scope.targets = vec!["*.Discord.COM".into(), "discord.com".into(), "geçersiz ad".into()];
        c.custom_methods = vec![
            CustomMethod { id: "custom".into(), name: "  ".into(), ..CustomMethod::default() },
            CustomMethod { id: "custom".into(), name: "İkinci".into(), ..CustomMethod::default() },
        ];
        let s = c.sanitized();
        assert_eq!(s.method, "default");
        assert_eq!(s.isp, "general");
        assert_eq!(s.scope.targets, vec!["discord.com".to_owned()]);
        assert_eq!(s.custom_methods[0].name, "Özel");
        assert_ne!(s.custom_methods[0].id, s.custom_methods[1].id);
    }

    #[test]
    fn custom_dns_validation_messages() {
        let bad = CustomDns { name: "X".into(), kind: CustomDnsKind::Udp, v4: "1.2.3".into(), ..CustomDns::default() };
        assert!(bad.validate().unwrap_err().contains("IPv4"));
        let doh = CustomDns {
            name: "D".into(),
            kind: CustomDnsKind::Doh,
            doh_url: "https://dns.example/dns-query".into(),
            bootstrap: "9.9.9.9, 2620:fe::fe".into(),
            ..CustomDns::default()
        };
        assert!(doh.validate().is_ok());
        assert_eq!(doh.bootstrap_ips().len(), 2);

        let mut c = AppConfig::default();
        c.dns.custom.push(CustomDns { id: "custom".into(), ..doh });
        c.dns.provider = "custom".into();
        assert!(matches!(c.active_dns(), ActiveDns::Encrypted { .. }));
        c.dns.mode = DnsMode::Redirect;
        assert!(c.validate().is_err(), "DoH girisi yonlendirme modunda kullanilamaz");
    }

    #[test]
    fn redirect_names() {
        let mut c = AppConfig::default();
        c.dns.mode = DnsMode::Redirect;
        c.dns.provider = "yandex".into();
        assert_eq!(c.active_dns().display_name(), "Yandex (1253)");
        c.dns.provider = "mullvad".into();
        assert!(c.validate().is_err());
        assert_eq!(c.active_dns(), ActiveDns::Off);
    }
}
