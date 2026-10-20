//! "En hizli DNS'i bul": her saglayiciya sifreli sorgular gonderip basari ve
//! gecikmeyi olcer. Yonetici gerektirmez (motorla ilgisi yok, dogrudan DoH).

use std::time::Instant;

use serde::Serialize;

use super::doh::DohClient;
use super::message;

pub const NAMES: &[&str] = &["discord.com", "google.com", "youtube.com", "wikipedia.org", "github.com"];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchResult {
    pub id: String,
    pub name: String,
    pub ok: u32,
    pub total: u32,
    /// Basarili sorgularin medyan suresi (ms); hic basari yoksa `None`.
    pub median_ms: Option<u32>,
    /// Ilk sorgu (TLS el sikismasi dahil) suresi.
    pub first_ms: Option<u32>,
    pub error: Option<String>,
}

pub fn query_packet(name: &str, qtype: u16) -> Vec<u8> {
    let mut m = vec![0, 0, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        m.push(label.len() as u8);
        m.extend_from_slice(label.as_bytes());
    }
    m.extend_from_slice(&[0, (qtype >> 8) as u8, qtype as u8, 0, 1]);
    m
}

pub fn run_one(id: &str, name: &str, url: &str, bootstrap: &[std::net::IpAddr]) -> BenchResult {
    let mut r = BenchResult { id: id.into(), name: name.into(), ok: 0, total: 0, median_ms: None, first_ms: None, error: None };
    let client = match DohClient::new(url, bootstrap) {
        Ok(c) => c,
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    };
    let mut times = Vec::new();
    for (i, n) in NAMES.iter().enumerate() {
        r.total += 1;
        let t = Instant::now();
        match client.resolve(&query_packet(n, 1)) {
            Ok(resp) if message::rcode(&resp) == Some(0) && message::min_answer_ttl(&resp).is_some() => {
                let ms = t.elapsed().as_millis() as u32;
                if i == 0 {
                    r.first_ms = Some(ms);
                } else {
                    times.push(ms);
                }
                r.ok += 1;
            }
            Ok(_) => {
                r.error.get_or_insert_with(|| "geçersiz yanıt".into());
            }
            Err(e) => {
                r.error.get_or_insert(e);
            }
        }
    }
    times.sort_unstable();
    r.median_ms = times.get(times.len() / 2).copied().or(r.first_ms);
    if r.ok == r.total {
        r.error = None;
    }
    r
}

/// Tum hazir saglayicilari paralel olcer (her biri kendi is parcaciginda).
pub fn run_all() -> Vec<BenchResult> {
    let handles: Vec<_> = shared::dns::PRESETS
        .iter()
        .filter(|p| p.supports_doh())
        .map(|p| {
            let bootstrap: Vec<std::net::IpAddr> = p.bootstrap_v4.iter().chain(p.bootstrap_v6).filter_map(|s| s.parse().ok()).collect();
            let (id, name, url) = (p.id, p.name, p.doh_url);
            std::thread::Builder::new().stack_size(256 * 1024).spawn(move || run_one(id, name, url, &bootstrap)).unwrap()
        })
        .collect();
    let mut results: Vec<BenchResult> = handles.into_iter().filter_map(|h| h.join().ok()).collect();
    results.sort_by_key(|r| (r.ok != r.total, r.median_ms.unwrap_or(u32::MAX)));
    results
}
