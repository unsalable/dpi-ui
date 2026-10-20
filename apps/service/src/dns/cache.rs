//! Sinirli DNS onbellegi. En fazla `max` kayit; doluysa once suresi dolanlar, sonra
//! en eski kullanilanlarin onda biri atilir. Bellek ust siniri ~0.5 MB.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const MIN_TTL: u32 = 30;
pub const MAX_TTL: u32 = 3600;
pub const NEGATIVE_TTL: u32 = 30;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    name: Box<str>,
    qtype: u16,
    qclass: u16,
}

struct Entry {
    response: Box<[u8]>,
    expires: Instant,
    used: Instant,
}

pub struct Cache {
    map: HashMap<Key, Entry>,
    max: usize,
}

impl Cache {
    pub fn new(max: usize) -> Self {
        Self { map: HashMap::with_capacity(max.min(256)), max }
    }

    pub fn get(&mut self, name: &str, qtype: u16, qclass: u16, now: Instant) -> Option<Vec<u8>> {
        let key = Key { name: name.into(), qtype, qclass };
        match self.map.get_mut(&key) {
            Some(e) if e.expires > now => {
                e.used = now;
                Some(e.response.to_vec())
            }
            Some(_) => {
                self.map.remove(&key);
                None
            }
            None => None,
        }
    }

    pub fn put(&mut self, name: &str, qtype: u16, qclass: u16, response: &[u8], ttl: u32, now: Instant) {
        if response.len() > 4096 {
            return; // nadir buyuk yanitlar onbellegi sisirmesin
        }
        if self.map.len() >= self.max {
            self.map.retain(|_, e| e.expires > now);
        }
        if self.map.len() >= self.max {
            let mut by_age: Vec<(Instant, Key)> = self.map.iter().map(|(k, e)| (e.used, k.clone())).collect();
            by_age.sort_by_key(|(t, _)| *t);
            for (_, k) in by_age.into_iter().take((self.max / 10).max(1)) {
                self.map.remove(&k);
            }
        }
        let ttl = ttl.clamp(MIN_TTL, MAX_TTL);
        self.map.insert(
            Key { name: name.into(), qtype, qclass },
            Entry { response: response.into(), expires: now + Duration::from_secs(ttl as u64), used: now },
        );
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_and_stays_bounded() {
        let mut c = Cache::new(20);
        let t0 = Instant::now();
        c.put("a.example", 1, 1, b"resp", 60, t0);
        assert_eq!(c.get("a.example", 1, 1, t0 + Duration::from_secs(10)).as_deref(), Some(&b"resp"[..]));
        assert!(c.get("a.example", 28, 1, t0).is_none());
        assert!(c.get("a.example", 1, 1, t0 + Duration::from_secs(61)).is_none(), "suresi doldu");
        c.put("b.example", 1, 1, b"x", 1, t0);
        assert!(c.get("b.example", 1, 1, t0 + Duration::from_secs(20)).is_some(), "TTL en az 30 sn");
        for i in 0..100 {
            c.put(&format!("h{i}.example"), 1, 1, b"x", 300, t0 + Duration::from_millis(i));
        }
        assert!(c.len() <= 20);
        assert!(c.get("h99.example", 1, 1, t0 + Duration::from_secs(1)).is_some(), "en yeni kalir");
    }
}
