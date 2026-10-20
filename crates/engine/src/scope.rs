//! Hangi baglantilara atlatma uygulanacagi (kapsam).
//!
//! - **Akilli** (varsayilan): yalnizca hedef listedeki (hazir + kullanici) ve
//!   ogrenilmis alan adlari. Listede olmayan bir alan adi izlenir; ClientHello'su
//!   yanitsiz kalip yeniden gonderilirse ya da sunucudan RST gelirse ogrenilir.
//! - **Tumu**: her baglanti.
//! Her iki modda da istisna listesi (asla dokunma) onceliklidir.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScopeMode {
    #[default]
    Smart,
    All,
}

/// Alan adi kumesi; bir ad kendisi ya da herhangi bir ust alan adi listedeyse eslesir.
#[derive(Clone, Debug, Default)]
pub struct HostSet {
    names: HashSet<Box<str>>,
}

impl HostSet {
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut set = Self::default();
        for n in names {
            set.insert(n.as_ref());
        }
        set
    }

    /// Adi normallestirip ekler. Gecersizse false.
    pub fn insert(&mut self, name: &str) -> bool {
        match normalize(name) {
            Some(n) => self.names.insert(n.into_boxed_str()),
            None => false,
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        normalize(name).is_some_and(|n| self.names.remove(n.as_str()))
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(|b| b.as_ref())
    }

    /// `host` ya da ust alan adlarindan biri kumede mi? (Buyuk/kucuk harf duyarsiz.)
    pub fn matches(&self, host: &str) -> bool {
        if self.names.is_empty() {
            return false;
        }
        // Cogu ad zaten kucuk harf; degilse tek seferlik kopya.
        let lowered;
        let mut h = if host.bytes().any(|b| b.is_ascii_uppercase()) {
            lowered = host.to_ascii_lowercase();
            lowered.as_str()
        } else {
            host
        };
        h = h.trim_end_matches('.');
        loop {
            if self.names.contains(h) {
                return true;
            }
            match h.split_once('.') {
                Some((_, rest)) => h = rest,
                None => return false,
            }
        }
    }
}

/// Kucuk harfe cevirir, "*." ve sondaki noktayi atar, gecerliligi denetler.
pub fn normalize(name: &str) -> Option<String> {
    let n = name.trim().trim_start_matches("*.").trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
    let ok = !n.is_empty()
        && n.len() <= 253
        && n.contains('.')
        && n.split('.').all(|l| !l.is_empty() && l.len() <= 63)
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_');
    ok.then_some(n)
}

#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub mode: ScopeMode,
    /// Hazir liste + kullanicinin ekledikleri.
    pub targets: HostSet,
    /// Otomatik ogrenilenler.
    pub learned: HostSet,
    /// Asla dokunulmayacaklar.
    pub excluded: HostSet,
    /// Akilli modda otomatik ogrenme acik mi?
    pub learning: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision<'a> {
    Bypass,
    Pass,
    /// Akilli mod: listede degil, izle (ogrenme icin).
    Watch(&'a str),
}

impl Scope {
    pub fn all() -> Self {
        Self { mode: ScopeMode::All, ..Self::default() }
    }

    pub fn decide<'a>(&self, host: Option<&'a str>) -> Decision<'a> {
        // Ad bu pakette gorunmuyor (ornegin buyuk ClientHello'nun ikinci TCP paketinde):
        // hangi siteye gittigi bilinemez, guvenli taraf olarak uygula.
        let Some(host) = host else { return Decision::Bypass };
        if self.excluded.matches(host) {
            return Decision::Pass;
        }
        match self.mode {
            ScopeMode::All => Decision::Bypass,
            ScopeMode::Smart => {
                if self.targets.matches(host) || self.learned.matches(host) {
                    Decision::Bypass
                } else if self.learning {
                    Decision::Watch(host)
                } else {
                    Decision::Pass
                }
            }
        }
    }

    /// Akilli modda ogrenme icin gelen ServerHello / RST yakalanmali mi?
    pub fn observes_inbound(&self) -> bool {
        self.mode == ScopeMode::Smart && self.learning
    }
}

/// Turkiye'de engelli oldugu bilinen alan adlari (Ekim 2026). Kullanici ayarlardan
/// kategori bazinda kapatabilir; ogrenme gerisini tamamlar.
pub const BUILTIN_TARGETS: &[(&str, &[&str])] = &[
    (
        "discord",
        &[
            "discord.com",
            "discord.gg",
            "discord.co",
            "discord.media",
            "discord.gift",
            "discord.new",
            "discord.dev",
            "discordapp.com",
            "discordapp.net",
            "discordstatus.com",
            "dis.gd",
            "discord-attachments-uploads-prd.storage.googleapis.com",
        ],
    ),
    ("roblox", &["roblox.com", "rbxcdn.com", "rbx.com", "robloxlabs.com", "roblox.qq.com"]),
    ("wattpad", &["wattpad.com"]),
    (
        "vpn",
        &[
            "nordvpn.com",
            "expressvpn.com",
            "protonvpn.com",
            "proton.me",
            "privateinternetaccess.com",
            "purevpn.com",
            "hide.me",
            "surfshark.com",
            "cyberghostvpn.com",
        ],
    ),
];

pub fn builtin_targets(categories: &[&str]) -> impl Iterator<Item = &'static str> {
    let all = categories.is_empty();
    let wanted: Vec<String> = categories.iter().map(|c| c.to_ascii_lowercase()).collect();
    BUILTIN_TARGETS
        .iter()
        .filter(move |(cat, _)| all || wanted.iter().any(|w| w == cat))
        .flat_map(|(_, names)| names.iter().copied())
}
