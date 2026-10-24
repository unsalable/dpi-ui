//! "En hizli DNS'i bul": hazir sifreli DNS saglayicilarina paralel sorgu atip basari ve
//! gecikmeyi olcer (Windows servisindeki olcumun Mac karsiligi).

use std::time::Instant;

use dnsclient::doh::DohClient;
use dnsclient::message;
use shared::ipc::BenchEntry;

const NAMES: &[&str] = &["discord.com", "google.com", "youtube.com", "wikipedia.org", "github.com"];

fn run_one(id: &str, name: &str, url: &str, bootstrap: &[std::net::IpAddr]) -> BenchEntry {
    let mut r = BenchEntry { id: id.into(), name: name.into(), ok: 0, total: 0, median_ms: None, error: None };
    let client = match DohClient::new(url, bootstrap) {
        Ok(c) => c,
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    };
    let mut first = None;
    let mut times = Vec::new();
    for (i, n) in NAMES.iter().enumerate() {
        r.total += 1;
        let t = Instant::now();
        match client.resolve(&message::build_query(n, 1)) {
            Ok(resp) if message::rcode(&resp) == Some(0) && message::min_answer_ttl(&resp).is_some() => {
                let ms = t.elapsed().as_millis() as u32;
                // Ilk sorgu TLS el sikismasini da icerir; medyana katilmaz.
                if i == 0 { first = Some(ms) } else { times.push(ms) }
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
    r.median_ms = times.get(times.len() / 2).copied().or(first);
    if r.ok == r.total {
        r.error = None;
    }
    r
}

pub fn run_all() -> Vec<BenchEntry> {
    let en = shared::i18n::english();
    let handles: Vec<_> = shared::dns::PRESETS
        .iter()
        .filter(|p| p.supports_doh())
        .map(|p| {
            let (id, name, url) = (p.id, p.label(en), p.doh_url);
            let bootstrap: Vec<std::net::IpAddr> = p.bootstrap_v4.iter().chain(p.bootstrap_v6).filter_map(|s| s.parse().ok()).collect();
            std::thread::spawn(move || run_one(id, name, url, &bootstrap))
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}
