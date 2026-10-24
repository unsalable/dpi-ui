//! Proxy motorunun hazir yontemleri.
//!
//! Windows'taki sahte paket yontemleri (TTL, MD5, bozuk saglama) cekirdek duzeyinde paket
//! yazmayi gerektirir; proxy bunlari yapamaz. Buradakiler yalnizca kendi yazdigimiz veriyi
//! sekillendirir. "Ters sira" Windows'taki ayni adli yontemin (zapret multidisorder) proxy
//! karsiligidir: ortusme (seqovl) yok, ilk parca TTL 1 ile duser ve sonra yeniden gonderilir.
//!
//! Canli olcum (24.10.2026, Kablonet, perf/proxy-live-test.ps1): parcalamasiz istekte Discord,
//! Roblox ve Wattpad kapali; yalnizca TLS kaydi bolen iki yontem besini de aciyor. Ters sira,
//! ad bolme ve duz bolme tek basina yetmiyor. Bu yuzden varsayilan ve ilk denenen TLS kaydi.

use crate::desync::{Desync, Pos};

#[derive(Clone, Copy, Debug)]
pub struct Method {
    pub id: &'static str,
    pub name: &'static str,
    pub name_en: &'static str,
    pub description: &'static str,
    pub description_en: &'static str,
    build: fn() -> Desync,
}

impl Method {
    pub fn desync(&self) -> Desync {
        (self.build)()
    }

    pub fn label(&self, en: bool) -> &'static str {
        if en { self.name_en } else { self.name }
    }

    pub fn about(&self, en: bool) -> &'static str {
        if en { self.description_en } else { self.description }
    }
}

pub const DISORDER: Method = Method {
    id: "disorder",
    name: "Ters sıra",
    name_en: "Reverse order",
    description: "İstek 2. bayttan bölünür; ilk parça yolda düşüp sonra yeniden gider, sunucuya önce geri kalanı ulaşır.",
    description_en: "The request is split at byte 2; the first part is dropped on the way and resent, so the rest arrives first.",
    build: || Desync { splits: vec![Pos::Byte(2)], disorder: true, ..Desync::default() },
};

pub const DISORDER_SNI: Method = Method {
    id: "disordersni",
    name: "Ters sıra + ad bölme",
    name_en: "Reverse order + name split",
    description: "Ters sıraya ek olarak site adının ortasından da bölünür.",
    description_en: "Reverse order, plus a split in the middle of the site name.",
    build: || Desync { splits: vec![Pos::Byte(2), Pos::HostMid], disorder: true, host_case: true, ..Desync::default() },
};

pub const TLS_RECORD: Method = Method {
    id: "tlsrec",
    name: "TLS kaydı bölme",
    name_en: "TLS record split",
    description: "İstek, site adının ortasından iki TLS kaydına ayrılır ve iki parça halinde gider. Gecikme eklemez.",
    description_en: "The request is split into two TLS records in the middle of the site name and sent in two parts. Adds no delay.",
    build: || Desync { tls_record: Some(Pos::HostMid), splits: vec![Pos::HostMid], host_case: true, ..Desync::default() },
};

pub const TLS_RECORD_DISORDER: Method = Method {
    id: "tlsrecdisorder",
    name: "TLS kaydı + ters sıra",
    name_en: "TLS record + reverse order",
    description: "TLS kaydı bölmeyle ters sıra birlikte.",
    description_en: "TLS record split and reverse order together.",
    build: || Desync {
        tls_record: Some(Pos::HostMid),
        splits: vec![Pos::Byte(2), Pos::HostMid],
        disorder: true,
        host_case: true,
    },
};

pub const SPLIT_SNI: Method = Method {
    id: "splitsni",
    name: "Ad bölme",
    name_en: "Name split",
    description: "İstek site adının ortasından iki parça halinde, normal sırada gider.",
    description_en: "The request goes in two parts, split in the middle of the site name, in normal order.",
    build: || Desync { splits: vec![Pos::HostMid], host_case: true, ..Desync::default() },
};

pub const SPLIT: Method = Method {
    id: "split",
    name: "Düz bölme",
    name_en: "Plain split",
    description: "İsteğin ilk baytı ayrı gider. En hafifi; çoğu sağlayıcıda tek başına yetmez.",
    description_en: "The first byte of the request goes separately. The lightest; on most ISPs it isn't enough alone.",
    build: || Desync { splits: vec![Pos::Byte(1)], ..Desync::default() },
};

/// Listedeki sira "Otomatik ayar bul"un deneme sirasidir.
pub const METHODS: &[Method] = &[TLS_RECORD, TLS_RECORD_DISORDER, DISORDER, DISORDER_SNI, SPLIT_SNI, SPLIT];

pub const DEFAULT_ID: &str = TLS_RECORD.id;

pub fn method(id: &str) -> Option<&'static Method> {
    METHODS.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_every_method_changes_a_hello() {
        let hello = engine::fakes::build_client_hello("discord.com", 517, false);
        for (i, m) in METHODS.iter().enumerate() {
            assert!(METHODS[i + 1..].iter().all(|o| o.id != m.id), "{}", m.id);
            assert!(!crate::desync::plan(&hello, &m.desync()).is_plain(), "{}", m.id);
            assert!(!m.about(true).is_empty() && !m.label(false).is_empty());
        }
        assert!(method(DEFAULT_ID).is_some());
    }
}
