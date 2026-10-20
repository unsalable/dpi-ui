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
    pub description: &'static str,
    build: fn() -> DpiConfig,
}

impl Method {
    pub fn config(&self) -> DpiConfig {
        (self.build)()
    }
}

pub const DEFAULT: Method = Method {
    id: "default",
    name: "Varsayılan",
    description: "Sahte paket (otomatik TTL) + istek bölme + QUIC engeli + Discord ses desteği.",
    build: DpiConfig::default,
};

pub const FIXED_TTL: Method = Method {
    id: "fixedttl",
    name: "Sabit TTL",
    description: "Sahte paket her zaman TTL 5 ile gider (GoodbyeDPI-Turkey \"-5 --set-ttl 5\" karşılığı).",
    build: || DpiConfig { auto_ttl: false, split_sni: false, ..DpiConfig::default() },
};

/// zapret2 "multidisorder:pos=2:seqovl=1": sahte paket yok, TTL tahmini gerekmez.
pub const DISORDER: Method = Method {
    id: "disorder",
    name: "Ters sıra",
    description: "Sahte paket yok: istek 2. bayttan bölünür, parçalar ters sırada ve 1 bayt örtüşmeyle gider.",
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
    description: "Sahte paket TTL 4 ile gider, istek bölünmez (zapret fake ttl=4).",
    build: || DpiConfig { auto_ttl: false, ttl: 4, split_tls: false, ..DpiConfig::default() },
};

pub const FAKE_TTL3: Method = Method {
    id: "ttl3",
    name: "Sahte TTL 3",
    description: "Sahte paket TTL 3 ile gider, istek bölünmez (zapret fake ttl=3).",
    build: || DpiConfig { auto_ttl: false, ttl: 3, split_tls: false, ..DpiConfig::default() },
};

pub const MD5SIG: Method = Method {
    id: "md5sig",
    name: "MD5 imzası",
    description: "Sahte paket TCP MD5 imzasıyla gider; sunucu atar, DPI işler (zapret fake md5sig).",
    build: || DpiConfig { fake_ttl: false, fake_md5sig: true, split_tls: false, ..DpiConfig::default() },
};

pub const MD5_TTL3: Method = Method {
    id: "md5ttl3",
    name: "MD5 + TTL 3",
    description: "Sahte paket hem MD5 imzası hem TTL 3 ile gider (zapret fake md5sig ttl=3).",
    build: || DpiConfig { auto_ttl: false, ttl: 3, fake_md5sig: true, split_tls: false, ..DpiConfig::default() },
};

pub const SPLIT_FAKE_TTL5: Method = Method {
    id: "fakesplit5",
    name: "Bölünmüş sahte",
    description: "Sahte istek 2. bayttan iki parça halinde TTL 5 ile gider, gerçek istek değişmez.",
    build: || DpiConfig { auto_ttl: false, ttl: 5, split_fake: true, split_tls: false, ..DpiConfig::default() },
};

pub const ZERO_FAKE: Method = Method {
    id: "zerofake",
    name: "Boş sahte",
    description: "Sahte paket olarak 4 sıfır bayt TTL 5 ile gider; DPI akışı tanıyamaz.",
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
    description: "Sahte paket yok: istek 2. bayttan bölünür, parçalar normal sırada gider.",
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
    description: "Sahte paket bozuk sağlama ve SEQ ile gider (GoodbyeDPI -9). TTL yöntemi işe yaramazsa dene.",
    build: || DpiConfig { fake_ttl: false, fake_wrong_checksum: true, fake_wrong_seq: true, ..DpiConfig::default() },
};

/// GoodbyeDPI -6: sahte paket gecmiste kalan SEQ ile gider (TTL'e dokunmaz).
pub const WRONG_SEQ: Method = Method {
    id: "wrongseq",
    name: "Yanlış SEQ",
    description: "Sahte paket geçersiz sıra numarasıyla gider; sunucu atar (GoodbyeDPI -6).",
    build: || DpiConfig { fake_ttl: false, fake_wrong_seq: true, ..DpiConfig::default() },
};

/// GoodbyeDPI -7: sahte paket yalnizca bozuk saglamayla gider.
pub const BAD_CHECKSUM: Method = Method {
    id: "badsum",
    name: "Bozuk sağlama",
    description: "Sahte paket yalnızca bozuk TCP sağlamasıyla gider (GoodbyeDPI -7).",
    build: || DpiConfig { fake_ttl: false, fake_wrong_checksum: true, ..DpiConfig::default() },
};

/// GoodbyeDPI "-5 --set-ttl 3": TTL 3 sahte paket + bolme.
pub const TTL3_SPLIT: Method = Method {
    id: "ttl3split",
    name: "TTL 3 + bölme",
    description: "Sahte paket TTL 3 ile gider, istek ters sırada bölünür (GoodbyeDPI -5 --set-ttl 3).",
    build: || DpiConfig { auto_ttl: false, ttl: 3, split_sni: false, ..DpiConfig::default() },
};

/// GoodbyeDPI "-5 --set-ttl 4".
pub const TTL4_SPLIT: Method = Method {
    id: "ttl4split",
    name: "TTL 4 + bölme",
    description: "Sahte paket TTL 4 ile gider, istek ters sırada bölünür (GoodbyeDPI -5 --set-ttl 4).",
    build: || DpiConfig { auto_ttl: false, ttl: 4, split_sni: false, ..DpiConfig::default() },
};

pub const SPLIT_ONLY: Method = Method {
    id: "split",
    name: "Sadece bölme",
    description: "Sahte paket yok, yalnızca istek bölünür. En hafifi; çoğu ISS'de tek başına yetmez.",
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
    pub description: &'static str,
    pub methods: &'static [&'static str],
}

impl Isp {
    pub fn recommended(&self) -> &'static Method {
        method(self.methods[0]).unwrap_or(&DEFAULT)
    }
}

pub const ISPS: &[Isp] = &[
    Isp {
        id: "general",
        name: "Genel",
        description: "Sağlayıcıya özel ayar yok; genel yöntemler listelenir.",
        methods: &["default", "fixedttl", "disorder", "ttl3split", "checksum", "split"],
    },
    Isp {
        id: "turktelekom",
        name: "Türk Telekom",
        description: "Önerilen: Ters sıra. Olmazsa Sahte TTL 4 / 3.",
        methods: &["disorder", "ttl4", "ttl3", "default"],
    },
    Isp {
        id: "superonline",
        name: "Superonline",
        description: "Önerilen: Ters sıra. Olmazsa MD5 imzası / MD5 + TTL 3 / Sahte TTL 3 / TTL 3 + bölme.",
        methods: &["disorder", "md5sig", "md5ttl3", "ttl3", "ttl3split", "checksum"],
    },
    Isp {
        id: "vodafone",
        name: "Vodafone",
        description: "Önerilen: Bölünmüş sahte (TTL 5). Olmazsa Ters sıra / Varsayılan.",
        methods: &["fakesplit5", "disorder", "default"],
    },
    Isp {
        id: "turknet",
        name: "TürkNet",
        description: "Önerilen: Varsayılan (otomatik TTL). Olmazsa Sabit TTL / Ters sıra.",
        methods: &["default", "fixedttl", "disorder", "ttl4split"],
    },
    Isp {
        id: "kablonet",
        name: "Kablonet",
        description: "Türksat Kablonet. Önerilen: Ters sıra. Olmazsa Sahte TTL 4 / Varsayılan.",
        methods: &["disorder", "ttl4", "default"],
    },
    Isp {
        id: "telekommobil",
        name: "TT Mobil",
        description: "Türk Telekom mobil hat / hotspot. Önerilen: Boş sahte (TTL 5).",
        methods: &["zerofake", "disorder", "ttl4"],
    },
    Isp {
        id: "turkcellmobil",
        name: "Turkcell Mobil",
        description: "Turkcell mobil hat / hotspot. Önerilen: Ters sıra.",
        methods: &["disorder", "default", "ttl3"],
    },
    Isp {
        id: "vodafonemobil",
        name: "Vodafone Mobil",
        description: "Vodafone mobil hat / hotspot. Önerilen: Ters sıra.",
        methods: &["disorder", "split2", "fakesplit5"],
    },
];

pub fn isp(id: &str) -> &'static Isp {
    ISPS.iter().find(|i| i.id.eq_ignore_ascii_case(id)).unwrap_or(&ISPS[0])
}
