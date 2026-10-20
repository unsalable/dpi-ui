//! Motor davranis ayarlari. JSON adlari v1'deki `NativeDpiConfig` ile birebir
//! ayni; boylece eski ayar dosyalarindaki ozel profiller oldugu gibi okunur.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Sahte istegin icerigi.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FakePayload {
    /// Engelsiz siteye (www.w3.org) ait gecerli gorunumlu TLS ClientHello / HTTP istegi.
    #[default]
    Tls,
    /// 4 sifir bayt: DPI akisi taniyamaz.
    Zeros,
}

// v1 bu alani sayi (0/1) olarak yaziyordu; hem sayiyi hem metni kabul ediyoruz.
impl<'de> Deserialize<'de> for FakePayload {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = FakePayload;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("0, 1, \"tls\" ya da \"zeros\"")
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<FakePayload, E> {
                Ok(if v == 1 { FakePayload::Zeros } else { FakePayload::Tls })
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<FakePayload, E> {
                self.visit_u64(v.max(0) as u64)
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<FakePayload, E> {
                Ok(if v.eq_ignore_ascii_case("zeros") { FakePayload::Zeros } else { FakePayload::Tls })
            }
        }
        d.deserialize_any(V)
    }
}

impl Serialize for FakePayload {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Self::Tls => "tls",
            Self::Zeros => "zeros",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DpiConfig {
    // ------------------------------------------------------------ sahte paket
    /// Gercek istekten once engelsiz bir siteye ait sahte istek gonder.
    pub fake_packet: bool,
    /// Sahte paket dusuk TTL ile gitsin (DPI gorur, sunucuya ulasmaz).
    pub fake_ttl: bool,
    /// TTL'i sunucunun SYN-ACK'indan olculen mesafeye gore hesapla.
    pub auto_ttl: bool,
    /// Sabit TTL (otomatik kapaliyken ya da olculemediginde).
    pub ttl: u8,
    /// Sahte paket bozuk TCP saglamasiyla gitsin.
    pub fake_wrong_checksum: bool,
    /// Sahte paket gecmiste kalan SEQ/ACK ile gitsin.
    pub fake_wrong_seq: bool,
    /// Sahte pakete TCP MD5 imzasi secenegi (RFC 2385) eklensin.
    #[serde(rename = "fakeMd5Sig")]
    pub fake_md5sig: bool,
    pub fake_payload: FakePayload,
    /// Sahte istegin kendisi de 2. bayttan iki parca halinde gitsin.
    pub split_fake: bool,
    /// Her sahte paket kac kez gonderilsin (1-10).
    pub fake_repeats: u8,

    // ------------------------------------------------------------------ bolme
    pub split_tls: bool,
    /// Parcalari ters sirada gonder.
    pub reverse_split: bool,
    /// Sabit bolme konumu (payload ofseti). 0 = sabit konumdan bolme.
    pub split_position: u16,
    /// SNI adinin ortasindan da bol.
    pub split_sni: bool,
    /// Sira ortusmesi (zapret seqovl) bayt sayisi. 0 = kapali.
    pub seq_overlap: u16,

    // ------------------------------------------------------------------ diger
    pub block_quic: bool,
    pub fragment_http: bool,
    /// Discord ses IP Discovery ve STUN mesajlarindan once sahte UDP gonder.
    pub voice_fake: bool,
    pub voice_fake_repeats: u8,
    /// 443 disinda TLS sayilacak portlar (or. Cloudflare 2053, 2083, 2087, 2096, 8443).
    pub extra_tls_ports: Vec<u16>,
}

impl Default for DpiConfig {
    /// GoodbyeDPI-Turkey "-5 --set-ttl 5" karsiligi (v1 varsayilani).
    fn default() -> Self {
        Self {
            fake_packet: true,
            fake_ttl: true,
            auto_ttl: true,
            ttl: 5,
            fake_wrong_checksum: false,
            fake_wrong_seq: false,
            fake_md5sig: false,
            fake_payload: FakePayload::Tls,
            split_fake: false,
            fake_repeats: 1,
            split_tls: true,
            reverse_split: true,
            split_position: 2,
            split_sni: true,
            seq_overlap: 0,
            block_quic: true,
            fragment_http: true,
            voice_fake: true,
            voice_fake_repeats: 6,
            extra_tls_ports: Vec::new(),
        }
    }
}

impl DpiConfig {
    /// Degerleri guvenli araliklara ceker (bozuk/el ile duzenlenmis dosyalara karsi).
    pub fn sanitized(mut self) -> Self {
        self.ttl = self.ttl.max(1);
        self.fake_repeats = self.fake_repeats.clamp(1, 10);
        self.voice_fake_repeats = self.voice_fake_repeats.clamp(1, 20);
        self.seq_overlap = self.seq_overlap.min(1000);
        self.extra_tls_ports.retain(|&p| p != 0 && p != 443 && p != 80 && p != 53);
        self.extra_tls_ports.sort_unstable();
        self.extra_tls_ports.dedup();
        self.extra_tls_ports.truncate(16);
        self
    }

    /// Sahte paketi koruyan en az bir yontem secili mi? Hicbiri yoksa motor TTL'e duser.
    pub fn has_fake_protection(&self) -> bool {
        self.fake_ttl || self.fake_wrong_checksum || self.fake_wrong_seq || self.fake_md5sig
    }

    /// Yalnizca yontemi ayirt eden kisim (sahte paket + bolme); listelerde tek satir.
    pub fn short_summary(&self) -> String {
        let full = self.summary();
        let parts: Vec<&str> = full.split(" · ").filter(|p| *p != "QUIC engeli" && *p != "Discord ses").collect();
        if parts.is_empty() { "Yalnızca QUIC engeli ve Discord ses desteği".to_owned() } else { parts.join(" · ") }
    }

    /// Listede gorunen kisa ozet.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(4);
        if self.fake_packet {
            let how = if self.auto_ttl && self.fake_ttl {
                "oto TTL".to_owned()
            } else if self.fake_ttl {
                format!("TTL {}", self.ttl)
            } else if self.fake_md5sig {
                "MD5".to_owned()
            } else if self.fake_wrong_checksum || self.fake_wrong_seq {
                "bozuk sağlama".to_owned()
            } else {
                "korumasız".to_owned()
            };
            let payload = if self.fake_payload == FakePayload::Zeros { "boş sahte" } else { "sahte paket" };
            parts.push(if self.split_fake { format!("{payload} ({how}, bölünmüş)") } else { format!("{payload} ({how})") });
        }
        if self.split_tls {
            let mut s = if self.reverse_split { "ters sıra bölme".to_owned() } else { "bölme".to_owned() };
            if self.seq_overlap > 0 {
                s.push_str(&format!(" +{} örtüşme", self.seq_overlap));
            }
            parts.push(s);
        }
        if self.block_quic {
            parts.push("QUIC engeli".to_owned());
        }
        if self.voice_fake {
            parts.push("Discord ses".to_owned());
        }
        if parts.is_empty() { "Atlatma tekniği seçilmedi.".to_owned() } else { parts.join(" · ") }
    }
}
