//! Hazir yontemler ve internet saglayicisi onerileri.
//!
//! Parametreler v1 ile ayni; v1 bunlari SplitWire-Turkey (cagritaskn) icindeki
//! zapret / zapret2 Turkiye ISS hazir ayarlarindan uyarlamisti. Canli testte TTL
//! tabanli profiller calisti; yanlis saglama/SEQ ve sahte paketsiz bolme bazi
//! aglar (sanal makine, mobil hotspot) icin duruyor.

use crate::config::{DpiConfig, FakePayload};

#[derive(Clone, Copy, Debug)]
pub struct Method {
    pub id: &'static str,
    pub name: &'static str,
    pub name_en: &'static str,
    pub description: &'static str,
    pub description_en: &'static str,
    build: fn() -> DpiConfig,
}

impl Method {
    pub fn config(&self) -> DpiConfig {
        (self.build)()
    }

    /// Secili dildeki ad (`en`: Ingilizce).
    pub fn label(&self, en: bool) -> &'static str {
        if en { self.name_en } else { self.name }
    }

    pub fn about(&self, en: bool) -> &'static str {
        if en { self.description_en } else { self.description }
    }
}

pub const DEFAULT: Method = Method {
    id: "default",
    name: "Varsayılan",
    name_en: "Default",
    description: "Sahte paket (otomatik TTL) + istek bölme + QUIC engeli + Discord ses desteği.",
    description_en: "Fake packet (auto TTL) + request split + QUIC block + Discord voice support.",
    build: DpiConfig::default,
};

pub const FIXED_TTL: Method = Method {
    id: "fixedttl",
    name: "Sabit TTL",
    name_en: "Fixed TTL",
    description: "Sahte paket her zaman TTL 5 ile gider (GoodbyeDPI-Turkey \"-5 --set-ttl 5\" karşılığı).",
    description_en: "The fake packet always uses TTL 5 (equivalent of GoodbyeDPI-Turkey \"-5 --set-ttl 5\").",
    build: || DpiConfig { auto_ttl: false, split_sni: false, ..DpiConfig::default() },
};

/// zapret2 "multidisorder:pos=2:seqovl=1": sahte paket yok, TTL tahmini gerekmez.
pub const DISORDER: Method = Method {
    id: "disorder",
    name: "Ters sıra",
    name_en: "Reverse order",
    description: "Sahte paket yok: istek 2. bayttan bölünür, parçalar ters sırada ve 1 bayt örtüşmeyle gider.",
    description_en: "No fake packet: the request is split at byte 2 and the parts go out in reverse order with a 1-byte overlap.",
    build: || DpiConfig {
        fake_packet: false,
        split_position: 2,
        split_sni: false,
        reverse_split: true,
        seq_overlap: 1,
        ..DpiConfig::default()
    },
};

pub const FAKE_TTL4: Method = Method {
    id: "ttl4",
    name: "Sahte TTL 4",
    name_en: "Fake TTL 4",
    description: "Sahte paket TTL 4 ile gider, istek bölünmez (zapret fake ttl=4).",
    description_en: "The fake packet uses TTL 4; the request isn't split (zapret fake ttl=4).",
    build: || DpiConfig { auto_ttl: false, ttl: 4, split_tls: false, ..DpiConfig::default() },
};

pub const FAKE_TTL3: Method = Method {
    id: "ttl3",
    name: "Sahte TTL 3",
    name_en: "Fake TTL 3",
    description: "Sahte paket TTL 3 ile gider, istek bölünmez (zapret fake ttl=3).",
    description_en: "The fake packet uses TTL 3; the request isn't split (zapret fake ttl=3).",
    build: || DpiConfig { auto_ttl: false, ttl: 3, split_tls: false, ..DpiConfig::default() },
};

pub const MD5SIG: Method = Method {
    id: "md5sig",
    name: "MD5 imzası",
    name_en: "MD5 signature",
    description: "Sahte paket TCP MD5 imzasıyla gider; sunucu atar, DPI işler (zapret fake md5sig).",
    description_en: "The fake packet carries a TCP MD5 signature; the server drops it, DPI processes it (zapret fake md5sig).",
    build: || DpiConfig { fake_ttl: false, fake_md5sig: true, split_tls: false, ..DpiConfig::default() },
};

pub const MD5_TTL3: Method = Method {
    id: "md5ttl3",
    name: "MD5 + TTL 3",
    name_en: "MD5 + TTL 3",
    description: "Sahte paket hem MD5 imzası hem TTL 3 ile gider (zapret fake md5sig ttl=3).",
    description_en: "The fake packet carries both an MD5 signature and TTL 3 (zapret fake md5sig ttl=3).",
    build: || DpiConfig { auto_ttl: false, ttl: 3, fake_md5sig: true, split_tls: false, ..DpiConfig::default() },
};

pub const SPLIT_FAKE_TTL5: Method = Method {
    id: "fakesplit5",
    name: "Bölünmüş sahte",
    name_en: "Split fake",
    description: "Sahte istek 2. bayttan iki parça halinde TTL 5 ile gider, gerçek istek değişmez.",
    description_en: "The fake request goes out in two parts split at byte 2 with TTL 5; the real request is unchanged.",
    build: || DpiConfig { auto_ttl: false, ttl: 5, split_fake: true, split_tls: false, ..DpiConfig::default() },
};

pub const ZERO_FAKE: Method = Method {
    id: "zerofake",
    name: "Boş sahte",
    name_en: "Empty fake",
    description: "Sahte paket olarak 4 sıfır bayt TTL 5 ile gider; DPI akışı tanıyamaz.",
    description_en: "Four zero bytes with TTL 5 are sent as the fake packet; DPI can't recognize the flow.",
    build: || DpiConfig {
        auto_ttl: false,
        ttl: 5,
        fake_payload: FakePayload::Zeros,
        split_tls: false,
        ..DpiConfig::default()
    },
};

pub const PLAIN_SPLIT: Method = Method {
    id: "split2",
    name: "Düz bölme",
    name_en: "Plain split",
    description: "Sahte paket yok: istek 2. bayttan bölünür, parçalar normal sırada gider.",
    description_en: "No fake packet: the request is split at byte 2 and the parts go out in normal order.",
    build: || DpiConfig {
        fake_packet: false,
        split_position: 2,
        split_sni: false,
        reverse_split: false,
        ..DpiConfig::default()
    },
};

pub const WRONG_CHECKSUM: Method = Method {
    id: "checksum",
    name: "Yanlış sağlama",
    name_en: "Wrong checksum",
    description: "Sahte paket bozuk sağlama ve SEQ ile gider (GoodbyeDPI -9). TTL yöntemi işe yaramazsa dene.",
    description_en: "The fake packet has a broken checksum and SEQ (GoodbyeDPI -9). Try it if the TTL method doesn't work.",
    build: || DpiConfig { fake_ttl: false, fake_wrong_checksum: true, fake_wrong_seq: true, ..DpiConfig::default() },
};

/// GoodbyeDPI -6: sahte paket gecmiste kalan SEQ ile gider (TTL'e dokunmaz).
pub const WRONG_SEQ: Method = Method {
    id: "wrongseq",
    name: "Yanlış SEQ",
    name_en: "Wrong SEQ",
    description: "Sahte paket geçersiz sıra numarasıyla gider; sunucu atar (GoodbyeDPI -6).",
    description_en: "The fake packet has an invalid sequence number; the server drops it (GoodbyeDPI -6).",
    build: || DpiConfig { fake_ttl: false, fake_wrong_seq: true, ..DpiConfig::default() },
};

/// GoodbyeDPI -7: sahte paket yalnizca bozuk saglamayla gider.
pub const BAD_CHECKSUM: Method = Method {
    id: "badsum",
    name: "Bozuk sağlama",
    name_en: "Bad checksum",
    description: "Sahte paket yalnızca bozuk TCP sağlamasıyla gider (GoodbyeDPI -7).",
    description_en: "The fake packet only has a broken TCP checksum (GoodbyeDPI -7).",
    build: || DpiConfig { fake_ttl: false, fake_wrong_checksum: true, ..DpiConfig::default() },
};

/// GoodbyeDPI "-5 --set-ttl 3": TTL 3 sahte paket + bolme.
pub const TTL3_SPLIT: Method = Method {
    id: "ttl3split",
    name: "TTL 3 + bölme",
    name_en: "TTL 3 + split",
    description: "Sahte paket TTL 3 ile gider, istek ters sırada bölünür (GoodbyeDPI -5 --set-ttl 3).",
    description_en: "The fake packet uses TTL 3; the request is split in reverse order (GoodbyeDPI -5 --set-ttl 3).",
    build: || DpiConfig { auto_ttl: false, ttl: 3, split_sni: false, ..DpiConfig::default() },
};

/// GoodbyeDPI "-5 --set-ttl 4".
pub const TTL4_SPLIT: Method = Method {
    id: "ttl4split",
    name: "TTL 4 + bölme",
    name_en: "TTL 4 + split",
    description: "Sahte paket TTL 4 ile gider, istek ters sırada bölünür (GoodbyeDPI -5 --set-ttl 4).",
    description_en: "The fake packet uses TTL 4; the request is split in reverse order (GoodbyeDPI -5 --set-ttl 4).",
    build: || DpiConfig { auto_ttl: false, ttl: 4, split_sni: false, ..DpiConfig::default() },
};

pub const SPLIT_ONLY: Method = Method {
    id: "split",
    name: "Sadece bölme",
    name_en: "Split only",
    description: "Sahte paket yok, yalnızca istek bölünür. En hafifi; çoğu ISS'de tek başına yetmez.",
    description_en: "No fake packet; the request is only split. The lightest one; on most ISPs it isn't enough on its own.",
    build: || DpiConfig { fake_packet: false, ..DpiConfig::default() },
};

pub const ALL: &[Method] = &[
    DEFAULT,
    FIXED_TTL,
    DISORDER,
    FAKE_TTL4,
    FAKE_TTL3,
    MD5SIG,
    MD5_TTL3,
    SPLIT_FAKE_TTL5,
    ZERO_FAKE,
    PLAIN_SPLIT,
    WRONG_CHECKSUM,
    TTL3_SPLIT,
    TTL4_SPLIT,
    WRONG_SEQ,
    BAD_CHECKSUM,
    SPLIT_ONLY,
];

pub fn method(id: &str) -> Option<&'static Method> {
    ALL.iter().find(|m| m.id.eq_ignore_ascii_case(id))
}

/// Internet saglayicisi: bilinen calisan yontemler oneri sirasiyla (ilki onerilen).
#[derive(Clone, Copy, Debug)]
pub struct Isp {
    pub id: &'static str,
    pub name: &'static str,
    pub name_en: &'static str,
    pub description: &'static str,
    pub description_en: &'static str,
    pub methods: &'static [&'static str],
}

impl Isp {
    /// Secili dildeki ad (`en`: Ingilizce).
    pub fn label(&self, en: bool) -> &'static str {
        if en { self.name_en } else { self.name }
    }

    pub fn about(&self, en: bool) -> &'static str {
        if en { self.description_en } else { self.description }
    }

    pub fn recommended(&self) -> &'static Method {
        method(self.methods[0]).unwrap_or(&DEFAULT)
    }
}

pub const ISPS: &[Isp] = &[
    Isp {
        id: "general",
        name: "Genel",
        name_en: "General",
        description: "Sağlayıcıya özel ayar yok; genel yöntemler listelenir.",
        description_en: "No ISP-specific settings; general methods are listed.",
        methods: &["default", "fixedttl", "disorder", "ttl3split", "checksum", "split"],
    },
    Isp {
        id: "turktelekom",
        name: "Türk Telekom",
        name_en: "Türk Telekom",
        description: "Önerilen: Ters sıra. Olmazsa Sahte TTL 4 / 3.",
        description_en: "Recommended: Reverse order. Otherwise Fake TTL 4 / 3.",
        methods: &["disorder", "ttl4", "ttl3", "default"],
    },
    Isp {
        id: "superonline",
        name: "Superonline",
        name_en: "Superonline",
        description: "Önerilen: Ters sıra. Olmazsa MD5 imzası / MD5 + TTL 3 / Sahte TTL 3 / TTL 3 + bölme.",
        description_en: "Recommended: Reverse order. Otherwise MD5 signature / MD5 + TTL 3 / Fake TTL 3 / TTL 3 + split.",
        methods: &["disorder", "md5sig", "md5ttl3", "ttl3", "ttl3split", "checksum"],
    },
    Isp {
        id: "vodafone",
        name: "Vodafone",
        name_en: "Vodafone",
        description: "Önerilen: Bölünmüş sahte (TTL 5). Olmazsa Ters sıra / Varsayılan.",
        description_en: "Recommended: Split fake (TTL 5). Otherwise Reverse order / Default.",
        methods: &["fakesplit5", "disorder", "default"],
    },
    Isp {
        id: "turknet",
        name: "TürkNet",
        name_en: "TürkNet",
        description: "Önerilen: Varsayılan (otomatik TTL). Olmazsa Sabit TTL / Ters sıra.",
        description_en: "Recommended: Default (auto TTL). Otherwise Fixed TTL / Reverse order.",
        methods: &["default", "fixedttl", "disorder", "ttl4split"],
    },
    Isp {
        id: "kablonet",
        name: "Kablonet",
        name_en: "Kablonet",
        description: "Türksat Kablonet. Önerilen: Ters sıra. Olmazsa Sahte TTL 4 / Varsayılan.",
        description_en: "Türksat Kablonet. Recommended: Reverse order. Otherwise Fake TTL 4 / Default.",
        methods: &["disorder", "ttl4", "default"],
    },
    Isp {
        id: "telekommobil",
        name: "TT Mobil",
        name_en: "TT Mobile",
        description: "Türk Telekom mobil hat / hotspot. Önerilen: Boş sahte (TTL 5).",
        description_en: "Türk Telekom mobile line / hotspot. Recommended: Empty fake (TTL 5).",
        methods: &["zerofake", "disorder", "ttl4"],
    },
    Isp {
        id: "turkcellmobil",
        name: "Turkcell Mobil",
        name_en: "Turkcell Mobile",
        description: "Turkcell mobil hat / hotspot. Önerilen: Ters sıra.",
        description_en: "Turkcell mobile line / hotspot. Recommended: Reverse order.",
        methods: &["disorder", "default", "ttl3"],
    },
    Isp {
        id: "vodafonemobil",
        name: "Vodafone Mobil",
        name_en: "Vodafone Mobile",
        description: "Vodafone mobil hat / hotspot. Önerilen: Ters sıra.",
        description_en: "Vodafone mobile line / hotspot. Recommended: Reverse order.",
        methods: &["disorder", "split2", "fakesplit5"],
    },
];

pub fn isp(id: &str) -> &'static Isp {
    ISPS.iter().find(|i| i.id.eq_ignore_ascii_case(id)).unwrap_or(&ISPS[0])
}
