//! Akilli liste: hedef / ogrenme / istisna davranislari.

mod common;

use common::*;
use engine::fakes;
use engine::scope::normalize;
use engine::{DnsMode, DpiConfig, Event, HostSet, LearnReason, Processor, Scope, ScopeMode, Verdict};

fn smart(targets: &[&str]) -> Scope {
    Scope { mode: ScopeMode::Smart, targets: HostSet::new(targets), learning: true, ..Scope::default() }
}

fn hello_to(host: &str) -> Vec<u8> {
    fakes::build_client_hello(host, 700, false)
}

#[test]
fn host_set_suffix_matching() {
    let set = HostSet::new(["discord.com", "*.Roblox.com.", "discordapp.net"]);
    assert!(set.matches("discord.com"));
    assert!(set.matches("gateway.discord.com"));
    assert!(set.matches("CDN.Roblox.com"));
    assert!(set.matches("media.discordapp.net."));
    assert!(!set.matches("notdiscord.com"));
    assert!(!set.matches("com"));
    assert!(!set.matches("discord.co"));
    assert_eq!(normalize("*.Discord.COM."), Some("discord.com".into()));
    assert_eq!(normalize("com"), None);
    assert_eq!(normalize("bad host.com"), None);
}

#[test]
fn smart_bypasses_targets_and_ignores_others() {
    let mut proc = Processor::new(DpiConfig::default(), DnsMode::Off, smart(&["discord.com"]));
    let mut p = out_tcp(false, 51000, 443, 1000, &hello_to("gateway.discord.com"));
    assert_eq!(run(&mut proc, &mut p, true, 0).0, Verdict::Replace);

    let mut q = out_tcp(false, 51001, 443, 2000, &hello_to("example.com"));
    assert_eq!(run(&mut proc, &mut q, true, 0).0, Verdict::Pass);
    assert_eq!(proc.stats().untouched, 1);
}

#[test]
fn smart_learns_from_retransmission_and_bypasses_it() {
    let mut proc = Processor::new(DpiConfig::default(), DnsMode::Off, smart(&[]));
    let hello = hello_to("blocked.example");
    let mut first = out_tcp(false, 51010, 443, 5000, &hello);
    assert_eq!(run(&mut proc, &mut first, true, 1_000).0, Verdict::Pass);

    // Cok erken ayni paket (ornegin iki kez yakalanma): yeniden gonderim sayilmaz.
    let mut early = out_tcp(false, 51010, 443, 5000, &hello);
    assert_eq!(run(&mut proc, &mut early, true, 1_050).0, Verdict::Pass);

    // ~1 sn sonra ayni SEQ'li ClientHello: sessiz engel -> ogren ve bu paketi atlat.
    let mut retx = out_tcp(false, 51010, 443, 5000, &hello);
    let (v, out) = run(&mut proc, &mut retx, true, 2_000);
    assert_eq!(v, Verdict::Replace);
    assert_eq!(reassemble(&out[1..], 5000), hello, "sunucu yine orijinal istegi alir");
    assert_eq!(
        proc.take_events(),
        vec![Event::Learned { host: "blocked.example".into(), reason: LearnReason::Retransmit }]
    );
    assert!(proc.scope().learned.matches("blocked.example"));

    // Sonraki baglanti dogrudan atlatilir.
    let mut next = out_tcp(false, 51011, 443, 9000, &hello);
    assert_eq!(run(&mut proc, &mut next, true, 3_000).0, Verdict::Replace);
}

#[test]
fn smart_learns_from_reset_but_not_after_server_hello() {
    let mut proc = Processor::new(DpiConfig::default(), DnsMode::Off, smart(&[]));

    let mut a = out_tcp(false, 51020, 443, 100, &hello_to("rst.example"));
    run(&mut proc, &mut a, true, 0);
    let mut rst = tcp(false, 443, 51020, 1, 1, 0x14, 50, &[], Addr { swap: true, ..Addr::default() });
    assert_eq!(run(&mut proc, &mut rst, false, 100).0, Verdict::Pass);
    assert!(proc.scope().learned.matches("rst.example"));

    let mut b = out_tcp(false, 51021, 443, 200, &hello_to("ok.example"));
    run(&mut proc, &mut b, true, 0);
    let server_hello = [0x16, 0x03, 0x03, 0x00, 0x50, 0x02, 0x00, 0x00, 0x4c, 0x03, 0x03];
    let mut sh = tcp(false, 443, 51021, 1, 1, 0x18, 50, &server_hello, Addr { swap: true, ..Addr::default() });
    run(&mut proc, &mut sh, false, 50);
    let mut late_rst = tcp(false, 443, 51021, 1, 1, 0x14, 50, &[], Addr { swap: true, ..Addr::default() });
    run(&mut proc, &mut late_rst, false, 5_000);
    assert!(!proc.scope().learned.matches("ok.example"), "ServerHello gelmisse RST ogretmez");
    assert_eq!(proc.stats().learned, 1);
}

#[test]
fn exclusions_win_and_learning_can_be_off() {
    let mut all = Scope::all();
    all.excluded.insert("bank.example");
    let mut proc = Processor::new(DpiConfig::default(), DnsMode::Off, all);
    let mut p = out_tcp(false, 51030, 443, 1, &hello_to("secure.bank.example"));
    assert_eq!(run(&mut proc, &mut p, true, 0).0, Verdict::Pass);
    let mut q = out_tcp(false, 51031, 443, 1, &hello_to("other.example"));
    assert_eq!(run(&mut proc, &mut q, true, 0).0, Verdict::Replace);

    let quiet = Scope { learning: false, ..smart(&[]) };
    let mut proc = Processor::new(DpiConfig::default(), DnsMode::Off, quiet);
    let hello = hello_to("x.example");
    let mut a = out_tcp(false, 51032, 443, 7, &hello);
    run(&mut proc, &mut a, true, 0);
    let mut b = out_tcp(false, 51032, 443, 7, &hello);
    assert_eq!(run(&mut proc, &mut b, true, 2_000).0, Verdict::Pass);
    assert!(!proc.build_filter().contains("tcp.Rst"));
}

#[test]
fn smart_filter_observes_server_replies() {
    let proc = Processor::new(DpiConfig::default(), DnsMode::Off, smart(&[]));
    let f = proc.build_filter();
    assert!(f.contains("tcp.Rst") && f.contains("tcp.Payload[5] == 0x02"));
}

#[test]
fn smart_http_uses_host_header() {
    let mut proc = Processor::new(DpiConfig::default(), DnsMode::Off, smart(&["discord.com"]));
    let mut p = out_tcp(false, 51040, 80, 1, b"GET / HTTP/1.1\r\nHost: discord.com\r\n\r\n");
    assert_eq!(run(&mut proc, &mut p, true, 0).0, Verdict::Replace);
    let mut q = out_tcp(false, 51041, 80, 1, b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n");
    assert_eq!(run(&mut proc, &mut q, true, 0).0, Verdict::Pass);
}

#[test]
fn watch_table_stays_bounded() {
    let mut proc = Processor::new(DpiConfig::default(), DnsMode::Off, smart(&[]));
    for i in 0..5000u32 {
        let host = format!("h{i}.example");
        let mut p = out_tcp(false, (10000 + i % 50000) as u16, 443, i, &hello_to(&host));
        run(&mut proc, &mut p, true, u64::from(i));
    }
    // Tablo siniri 1024; asilirsa temizlenir. Burada yalnizca cokmedigini ve
    // ogrenme olmadigini dogruluyoruz.
    assert_eq!(proc.stats().learned, 0);
}
