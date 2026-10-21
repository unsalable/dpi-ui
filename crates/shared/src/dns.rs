//! Hazir DNS saglayicilari. Her biri sifreli (DoH) ve/veya duz UDP yonlendirme
//! icin gereken bilgiyi tasir. DoH'ta adres cozumlemesi gerekmesin diye baslangic
//! (bootstrap) IP'leri sabittir; TLS baglantisi bu IP'lere, SNI ise `doh_host`'a gider.

#[derive(Clone, Copy, Debug)]
pub struct DnsPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub name_en: &'static str,
    pub description: &'static str,
    pub description_en: &'static str,
    /// Sifreli DNS adresi: `https://...` (DoH) ya da `tls://host` (DoT). Bos degilse desteklenir.
    pub doh_url: &'static str,
    pub bootstrap_v4: &'static [&'static str],
    pub bootstrap_v6: &'static [&'static str],
    /// Port yonlendirme modunda kullanilacak adresler (port 53 disi olani anlamli).
    pub udp_v4: Option<(&'static str, u16)>,
    pub udp_v6: Option<(&'static str, u16)>,
}

impl DnsPreset {
    /// Secili dildeki ad (`en`: Ingilizce).
    pub fn label(&self, en: bool) -> &'static str {
        if en { self.name_en } else { self.name }
    }

    pub fn about(&self, en: bool) -> &'static str {
        if en { self.description_en } else { self.description }
    }

    pub fn supports_doh(&self) -> bool {
        !self.doh_url.is_empty()
    }

    pub fn supports_redirect(&self) -> bool {
        self.udp_v4.is_some() || self.udp_v6.is_some()
    }

    /// DoH URL'sinin ana bilgisayar adi (TLS SNI'si).
    pub fn doh_host(&self) -> &'static str {
        url_host(self.doh_url).unwrap_or("")
    }
}

pub const PRESETS: &[DnsPreset] = &[
    DnsPreset {
        id: "cloudflare",
        name: "Cloudflare",
        name_en: "Cloudflare",
        description: "Hızlı ve gizlilik odaklı.",
        description_en: "Fast and privacy-focused.",
        doh_url: "https://cloudflare-dns.com/dns-query",
        bootstrap_v4: &["1.1.1.1", "1.0.0.1"],
        bootstrap_v6: &["2606:4700:4700::1111", "2606:4700:4700::1001"],
        udp_v4: Some(("1.1.1.1", 53)),
        udp_v6: Some(("2606:4700:4700::1111", 53)),
    },
    DnsPreset {
        id: "google",
        name: "Google",
        name_en: "Google",
        description: "Yaygın ve kararlı.",
        description_en: "Widespread and stable.",
        doh_url: "https://dns.google/dns-query",
        bootstrap_v4: &["8.8.8.8", "8.8.4.4"],
        bootstrap_v6: &["2001:4860:4860::8888", "2001:4860:4860::8844"],
        udp_v4: Some(("8.8.8.8", 53)),
        udp_v6: Some(("2001:4860:4860::8888", 53)),
    },
    DnsPreset {
        id: "quad9",
        name: "Quad9",
        name_en: "Quad9",
        description: "Zararlı alan adlarını engeller.",
        description_en: "Blocks malicious domains.",
        doh_url: "tls://dns.quad9.net",
        bootstrap_v4: &["9.9.9.9", "149.112.112.112"],
        bootstrap_v6: &["2620:fe::fe", "2620:fe::9"],
        udp_v4: Some(("9.9.9.9", 53)),
        udp_v6: Some(("2620:fe::fe", 53)),
    },
    DnsPreset {
        id: "adguard",
        name: "AdGuard",
        name_en: "AdGuard",
        description: "Reklam ve izleyicileri engeller.",
        description_en: "Blocks ads and trackers.",
        doh_url: "https://dns.adguard-dns.com/dns-query",
        bootstrap_v4: &["94.140.14.14", "94.140.15.15"],
        bootstrap_v6: &["2a10:50c0::ad1:ff", "2a10:50c0::ad2:ff"],
        udp_v4: Some(("94.140.14.14", 53)),
        udp_v6: Some(("2a10:50c0::ad1:ff", 53)),
    },
    DnsPreset {
        id: "yandex",
        name: "Yandex",
        name_en: "Yandex",
        description: "Standart dışı 1253 portu ISS yönlendirmesini aşar.",
        description_en: "The non-standard port 1253 gets around ISP redirection.",
        doh_url: "tls://common.dot.dns.yandex.net",
        bootstrap_v4: &["77.88.8.8", "77.88.8.1"],
        bootstrap_v6: &["2a02:6b8::feed:0ff"],
        udp_v4: Some(("77.88.8.8", 1253)),
        udp_v6: Some(("2a02:6b8::feed:0ff", 1253)),
    },
    DnsPreset {
        id: "opendns",
        name: "OpenDNS",
        name_en: "OpenDNS",
        description: "Cisco. 5353 portuyla yönlendirme de yapılabilir.",
        description_en: "Cisco. Can also redirect via port 5353.",
        doh_url: "https://doh.opendns.com/dns-query",
        bootstrap_v4: &["208.67.222.222", "208.67.220.220"],
        bootstrap_v6: &["2620:119:35::35", "2620:119:53::53"],
        udp_v4: Some(("208.67.222.222", 5353)),
        udp_v6: Some(("2620:119:35::35", 5353)),
    },
    DnsPreset {
        id: "cloudflare-family",
        name: "Cloudflare Aile",
        name_en: "Cloudflare Family",
        description: "Zararlı ve yetişkin içerikli siteleri engeller.",
        description_en: "Blocks malicious and adult sites.",
        doh_url: "https://family.cloudflare-dns.com/dns-query",
        bootstrap_v4: &["1.1.1.3", "1.0.0.3"],
        bootstrap_v6: &["2606:4700:4700::1113", "2606:4700:4700::1003"],
        udp_v4: Some(("1.1.1.3", 53)),
        udp_v6: Some(("2606:4700:4700::1113", 53)),
    },
    DnsPreset {
        id: "mullvad",
        name: "Mullvad",
        name_en: "Mullvad",
        description: "Kayıt tutmayan, gizlilik odaklı.",
        description_en: "No logs, privacy-focused.",
        doh_url: "tls://dns.mullvad.net",
        bootstrap_v4: &["194.242.2.2"],
        bootstrap_v6: &["2a07:e340::2"],
        udp_v4: None,
        udp_v6: None,
    },
];

pub fn preset(id: &str) -> Option<&'static DnsPreset> {
    PRESETS.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// `https://host[:port]/yol` ya da `tls://host[:port]` icinden host'u cikarir.
pub fn url_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("tls://"))?;
    let authority = rest.split('/').next()?;
    let host = authority.rsplit_once(':').map_or(authority, |(h, p)| if p.bytes().all(|b| b.is_ascii_digit()) { h } else { authority });
    (!host.is_empty()).then_some(host)
}
