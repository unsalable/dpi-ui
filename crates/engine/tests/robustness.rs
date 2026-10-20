//! Bozuk, kesik ve rastgele paketler motoru asla cokertmemeli (panik yok) ve
//! tablolar sinirsiz buyumemeli. cargo-fuzz gece derleyicisi istedigi icin burada
//! deterministik bir rastgele ureteciyle kararli surumde calisan bir tarama var.

mod common;

use common::*;
use engine::{DnsMode, DpiConfig, HostSet, Output, Processor, Redirect, Scope, ScopeMode, fakes, presets};

/// xorshift64*: bagimliliksiz, tekrarlanabilir.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn processors() -> Vec<Processor> {
    let smart = Scope { mode: ScopeMode::Smart, targets: HostSet::new(["discord.com"]), learning: true, ..Scope::default() };
    let redirect = DnsMode::Redirect(Redirect { v4: Some(([77, 88, 8, 8], 1253)), v6: Some(([1; 16], 1253)) });
    let mut v: Vec<Processor> =
        presets::ALL.iter().map(|m| Processor::new(m.config(), DnsMode::Intercept, Scope::all())).collect();
    v.push(Processor::new(DpiConfig { seq_overlap: 7, split_fake: true, fake_repeats: 3, ..DpiConfig::default() }, redirect, smart));
    v
}

fn seeds() -> Vec<Vec<u8>> {
    let hello = fakes::build_client_hello("discord.com", 1800, true);
    vec![
        out_tcp(false, 50000, 443, 1, &hello[..1200]),
        out_tcp(true, 50000, 443, 1201, &hello[1200..]),
        out_tcp(false, 50001, 80, 9, b"GET / HTTP/1.1\r\nHost: discord.com\r\n\r\n"),
        udp(false, 50002, 50004, &discord_discovery(7), Addr::default()),
        udp(true, 50003, 3478, &stun_binding(4), Addr::default()),
        udp(false, 50004, 53, &[0xbe, 0xef, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 3, b'f', b'o', b'o', 0, 0, 1, 0, 1], Addr::default()),
        syn_ack(false, 50000, 116),
    ]
}

#[test]
fn random_and_mutated_packets_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut procs = processors();
    let seeds = seeds();
    let mut out = Output::with_capacity(4096);

    for round in 0..40_000u64 {
        let mut pkt = if rng.below(4) == 0 {
            let len = rng.below(1600);
            (0..len).map(|_| rng.next() as u8).collect::<Vec<u8>>()
        } else {
            let mut p = seeds[rng.below(seeds.len())].clone();
            for _ in 0..1 + rng.below(6) {
                match rng.below(4) {
                    0 if !p.is_empty() => {
                        let i = rng.below(p.len());
                        p[i] = rng.next() as u8;
                    }
                    1 if !p.is_empty() => {
                        let cut = rng.below(p.len());
                        p.truncate(cut);
                    }
                    2 => p.extend((0..rng.below(64)).map(|_| rng.next() as u8)),
                    _ if p.len() > 4 => {
                        // Uzunluk alanlarini bozmak en verimli hata kaynagi.
                        let i = [2usize, 3, 4, 5, 32, 44, 45, 60, 63, 64][rng.below(10)].min(p.len() - 1);
                        p[i] = rng.next() as u8;
                    }
                    _ => {}
                }
            }
            p
        };
        let which = rng.below(procs.len());
        let proc = &mut procs[which];
        let outbound = rng.below(5) != 0;
        let _ = proc.process(&mut pkt, outbound, round * 3, &mut out);
        for item in out.items() {
            // Uretilen her paket gecerli bir IP paketi olmali.
            assert!(engine::IpPacket::parse(out.resolve(item, &pkt)).is_some(), "gecersiz cikti, tur {round}");
        }
    }
}
