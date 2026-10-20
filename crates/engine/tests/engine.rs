//! v1 SelfTest.cs motor bolumlerinin birebir karsiliklari + v2 eklentileri.

mod common;

use common::*;
use engine::fakes::{self, FAKE_HOST};
use engine::presets;
use engine::processor::{compute_auto_ttl, is_first_wins_initial_ttl};
use engine::{DnsMode, DpiConfig, Processor, Redirect, Scope, Verdict, tls};

fn proc_with(cfg: DpiConfig) -> Processor {
    Processor::new(cfg, DnsMode::Off, Scope::all())
}

fn default_proc() -> Processor {
    proc_with(presets::DEFAULT.config())
}

const YANDEX_V6: [u8; 16] = [0x2a, 0x02, 0x06, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe, 0xed, 0x00, 0xff];

fn yandex() -> DnsMode {
    DnsMode::Redirect(Redirect { v4: Some(([77, 88, 8, 8], 1253)), v6: Some((YANDEX_V6, 1253)) })
}

// =========================================================== temel ayristirma

#[test]
fn fake_packet_content() {
    let hello = fakes::tls_client_hello();
    assert_eq!(hello.len(), 517);
    assert!(tls::is_client_hello(hello));
    assert_eq!(tls::sni_host(hello), Some(FAKE_HOST));
    assert!(std::str::from_utf8(fakes::http_request()).unwrap().starts_with("GET / HTTP/1.1\r\nHost: www.w3.org\r\n"));
}

#[test]
fn sni_parsing_and_hosts() {
    let hello = fakes::build_client_hello(BLOCKED, 700, false);
    let (off, len) = tls::find_sni(&hello).unwrap();
    assert_eq!(&hello[off..off + len], BLOCKED.as_bytes());
    assert_eq!(tls::sni_host(&hello), Some(BLOCKED));
    // Kesik paket: ad yarida -> host yok ama konum bulunur.
    let cut = &hello[..off + 3];
    assert_eq!(tls::find_sni(cut), Some((off, 3)));
    assert_eq!(tls::sni_host(cut), None);
    assert_eq!(tls::http_host(b"GET / HTTP/1.1\r\nHost: Discord.com:80\r\n\r\n"), Some("Discord.com"));
}

// =========================================================== motor

fn engine_tls_default(v6: bool) {
    let mut proc = default_proc();
    let hello = fakes::build_client_hello(BLOCKED, 1200, false);
    let mut packet = out_tcp(v6, 50000, 443, 777_000, &hello);
    let original = packet.clone();

    let (v, out) = run(&mut proc, &mut packet, true, 1000);
    assert_eq!(v, Verdict::Replace);
    assert_eq!(packet, original, "orijinal tampon degismemeli");
    assert!(out.len() >= 2);

    let fake = &out[0];
    let fip = fake.ip();
    assert_eq!(fake.payload(), fakes::tls_client_hello());
    assert!(!contains(&fake.data, BLOCKED.as_bytes()));
    assert_eq!(fip.ttl, 5, "SYN-ACK yok: sabit TTL");
    assert_eq!(fip.tcp_seq, 777_000);
    assert!(fake.recalc && !fake.corrupt);
    assert!(ip_len_matches(&fake.data, v6));

    let segs = &out[1..];
    assert_eq!(segs.len(), 3, "2. bayt + SNI ortasi");
    let seqs: Vec<u32> = segs.iter().map(|s| s.ip().tcp_seq).collect();
    let mut desc = seqs.clone();
    desc.sort_by(|a, b| b.cmp(a));
    assert_eq!(seqs, desc, "ters sirada");
    assert_eq!(reassemble(segs, 777_000), hello);
    assert!(segs.iter().all(|s| !contains(&s.data, BLOCKED.as_bytes())));
    assert!(segs.iter().all(|s| ip_len_matches(&s.data, v6)));
    assert!(segs.iter().all(|s| s.ip().ttl == 128));
    let smallest = segs.iter().map(|s| s.ip()).min_by_key(|i| i.tcp_seq).unwrap();
    assert_eq!(smallest.payload_len, 2);

    let mut app = out_tcp(v6, 50000, 443, 1, &[0x17, 0x03, 0x03, 0x00, 0x20, 0x01, 0x02, 0x03]);
    assert_eq!(run(&mut proc, &mut app, true, 1000).0, Verdict::Pass);
    let mut kx = out_tcp(v6, 50000, 443, 1, &[0x16, 0x03, 0x03, 0x00, 0x46, 0x10, 0x00, 0x00, 0x42]);
    assert_eq!(run(&mut proc, &mut kx, true, 1000).0, Verdict::Pass);
    let mut cont = out_tcp(v6, 50000, 443, 2, &[0x41; 300]);
    assert_eq!(run(&mut proc, &mut cont, true, 1000).0, Verdict::Pass);
}

#[test]
fn tls_default_v4() {
    engine_tls_default(false);
}

#[test]
fn tls_default_v6() {
    engine_tls_default(true);
}

#[test]
fn auto_ttl_math() {
    assert_eq!(compute_auto_ttl(55, 5), 5);
    assert_eq!(compute_auto_ttl(56, 5), 4);
    assert_eq!(compute_auto_ttl(121, 5), 4);
    assert_eq!(compute_auto_ttl(60, 5), 2);
    assert_eq!(compute_auto_ttl(62, 5), 0);
    assert_eq!(compute_auto_ttl(108, 5), 10);
    assert_eq!(compute_auto_ttl(243, 5), 8);
    assert_eq!(compute_auto_ttl(20, 5), 5);
}

#[test]
fn auto_ttl_flows() {
    for v6 in [false, true] {
        let mut proc = default_proc();
        let mut syn = tcp(v6, 443, 50001, 5000, 778_000, 0x12, 52, &[], Addr { swap: true, ..Addr::default() });
        assert_eq!(run(&mut proc, &mut syn, false, 1000).0, Verdict::Pass);
        let mut hello = tcp(v6, 50001, 443, 778_000, 5001, 0x18, 128, &fakes::build_client_hello(BLOCKED, 600, false), Addr::default());
        let (_, out) = run(&mut proc, &mut hello, true, 1050);
        assert_eq!(out[0].ip().ttl, 8, "12 hop -> TTL 8");

        let mut near = default_proc();
        let mut s = tcp(v6, 443, 50002, 1, 900, 0x12, 62, &[], Addr { swap: true, ..Addr::default() });
        run(&mut near, &mut s, false, 1000);
        let mut h = tcp(v6, 50002, 443, 900, 2, 0x18, 128, &fakes::build_client_hello(BLOCKED, 600, false), Addr::default());
        let (_, out) = run(&mut near, &mut h, true, 1010);
        assert!(!out.is_empty() && out.iter().all(|o| !contains(&o.data, FAKE_HOST.as_bytes())), "yakin sunucuda sahte yok");

        let mut other = default_proc();
        let mut s = tcp(v6, 443, 59999, 1, 1, 0x12, 62, &[], Addr { swap: true, ..Addr::default() });
        run(&mut other, &mut s, false, 1000);
        let mut h = tcp(v6, 50003, 443, 1, 2, 0x18, 128, &fakes::build_client_hello(BLOCKED, 600, false), Addr::default());
        let (_, out) = run(&mut other, &mut h, true, 1010);
        assert_eq!(out[0].ip().ttl, 5, "baska akisin kaydi karismaz");
    }
}

#[test]
fn profiles() {
    let hello = fakes::build_client_hello(BLOCKED, 700, false);

    let mut cks = proc_with(presets::WRONG_CHECKSUM.config());
    let mut p1 = tcp(false, 50010, 443, 100_000, 90_000, 0x18, 128, &hello, Addr::default());
    let (_, out) = run(&mut cks, &mut p1, true, 0);
    let f = out[0].ip();
    assert_eq!(out.iter().filter(|o| contains(&o.data, FAKE_HOST.as_bytes())).count(), 1);
    assert!(out[0].corrupt);
    assert_eq!(f.ttl, 128);
    assert_eq!(f.tcp_seq, 90_000);
    assert_eq!(f.tcp_ack, 24_000);

    let mut split = proc_with(presets::SPLIT_ONLY.config());
    let mut p2 = out_tcp(false, 50011, 443, 5, &hello);
    let (_, out) = run(&mut split, &mut p2, true, 0);
    assert!(out.iter().all(|o| !contains(&o.data, FAKE_HOST.as_bytes())));
    assert_eq!(out.len(), 3);
    assert_eq!(reassemble(&out, 5), hello);

    let unsafe_cfg =
        DpiConfig { fake_ttl: false, fake_wrong_checksum: false, fake_wrong_seq: false, split_tls: false, ..DpiConfig::default() };
    let mut up = proc_with(unsafe_cfg);
    let mut p3 = out_tcp(false, 50012, 443, 5, &hello);
    let (_, out) = run(&mut up, &mut p3, true, 0);
    assert!(out.len() == 2 && out[0].ip().ttl == 5, "korumasiz: TTL'e duser");
    assert!(out[1].original && !out[1].recalc);

    let mut legacy = proc_with(DpiConfig { split_position: 0, ..DpiConfig::default() });
    let mut p4 = out_tcp(false, 50013, 443, 5, &hello);
    assert_eq!(run(&mut legacy, &mut p4, true, 0).1.len(), 3, "splitPosition=0: sahte + 2 parca");

    let none = proc_with(DpiConfig {
        fake_packet: false,
        split_tls: false,
        block_quic: false,
        fragment_http: false,
        voice_fake: false,
        ..DpiConfig::default()
    });
    assert_eq!(none.build_filter(), "false");
}

#[test]
fn seq_overlap() {
    let hello = fakes::build_client_hello(BLOCKED, 700, false);

    let mut proc = proc_with(presets::DISORDER.config());
    let mut p = out_tcp(false, 50100, 443, 900_000, &hello);
    let (v, out) = run(&mut proc, &mut p, true, 0);
    assert_eq!(v, Verdict::Replace);
    assert!(out.iter().all(|o| !contains(&o.data, FAKE_HOST.as_bytes())));
    assert_eq!(out.len(), 2);
    let first = out[0].ip();
    let second = out[1].ip();
    assert_eq!(first.tcp_seq, 900_001, "1 bayt ortusme");
    assert_eq!(out[0].data[first.payload_offset], 0);
    assert_eq!(first.payload_len, hello.len() - 1);
    assert_eq!(&out[0].data[first.payload_offset + 1..], &hello[2..]);
    assert!(second.tcp_seq == 900_000 && second.payload_len == 2);
    assert!(out.iter().all(|o| o.data.len() <= p.len()));
    assert_eq!(receiver_view(&out, 900_000), hello);
    let dpi = first_wins_view(&out, 900_000);
    assert!(!tls::is_client_hello(&dpi), "ilk geleni tutan DPI ClientHello goremez");

    for v6 in [false, true] {
        let mut win = proc_with(presets::DISORDER.config());
        let mut ws = tcp(v6, 443, 50104, 1, 910_000, 0x12, 116, &[], Addr { swap: true, ..Addr::default() });
        run(&mut win, &mut ws, false, 1000);
        let mut wh = tcp(v6, 50104, 443, 910_000, 2, 0x18, 128, &hello, Addr::default());
        let (_, out) = run(&mut win, &mut wh, true, 1010);
        assert!(out.len() == 2 && out[0].ip().tcp_seq == 910_002, "Windows sunucu: ortusmesiz ters sira");
        assert_eq!(first_wins_view(&out, 910_000), hello);
        assert_eq!(receiver_view(&out, 910_000), hello);
        assert_eq!(win.stats().overlap_skipped, 1);

        let mut linux = proc_with(presets::DISORDER.config());
        let mut ls = tcp(v6, 443, 50105, 1, 920_000, 0x12, 52, &[], Addr { swap: true, ..Addr::default() });
        run(&mut linux, &mut ls, false, 1000);
        let mut lh = tcp(v6, 50105, 443, 920_000, 2, 0x18, 128, &hello, Addr::default());
        let (_, out) = run(&mut linux, &mut lh, true, 1010);
        assert!(out.len() == 2 && out[0].ip().tcp_seq == 920_001, "Linux sunucu: ortusme korunur");
        assert_eq!(receiver_view(&out, 920_000), hello);
    }

    assert!(!is_first_wins_initial_ttl(64));
    assert!(is_first_wins_initial_ttl(65));
    assert!(is_first_wins_initial_ttl(243));

    let mut fwd =
        proc_with(DpiConfig { fake_packet: false, split_sni: false, reverse_split: false, seq_overlap: 3, ..DpiConfig::default() });
    let mut p2 = out_tcp(false, 50101, 443, 800_000, &hello);
    let (_, out) = run(&mut fwd, &mut p2, true, 0);
    let f0 = out[0].ip();
    assert!(f0.tcp_seq == 799_997 && f0.payload_len == 5 && out[0].data[f0.payload_offset + 3] == hello[0]);
    assert_eq!(receiver_view(&out, 800_000), hello);

    let mut cancel = proc_with(DpiConfig { fake_packet: false, split_sni: false, seq_overlap: 2, ..DpiConfig::default() });
    let mut p3 = out_tcp(false, 50102, 443, 700_000, &hello);
    let (_, out) = run(&mut cancel, &mut p3, true, 0);
    assert_eq!(out[0].ip().tcp_seq, 700_002, "ortusme >= bolme konumu: iptal");
    assert_eq!(reassemble(&out, 700_000), hello);

    let mut multi = proc_with(DpiConfig { fake_packet: false, seq_overlap: 1, ..DpiConfig::default() });
    let mut p4 = out_tcp(true, 50103, 443, 600_000, &hello);
    let (_, out) = run(&mut multi, &mut p4, true, 0);
    assert_eq!(out.len(), 3);
    assert_eq!(receiver_view(&out, 600_000), hello);
    assert!(out.iter().all(|o| !contains(&o.data, BLOCKED.as_bytes())));
}

#[test]
fn fake_variants() {
    let hello = fakes::build_client_hello(BLOCKED, 700, false);

    let mut md5 = proc_with(presets::MD5SIG.config());
    let mut p1 = tcp(false, 50110, 443, 10_000, 20_000, 0x18, 128, &hello, Addr::default());
    let (_, out) = run(&mut md5, &mut p1, true, 0);
    assert!(out.len() == 2 && out[1].original && !out[1].recalc);
    let m = out[0].ip();
    assert_eq!(m.l4_header_len(), 40);
    assert_eq!(&out[0].data[m.l4_offset + 20..m.l4_offset + 24], &[1, 1, 19, 18]);
    assert_eq!(&out[0].data[m.payload_offset..], fakes::tls_client_hello());
    assert!(ip_len_matches(&out[0].data, false));
    assert!(m.ttl == 128 && m.tcp_seq == 10_000 && !out[0].corrupt);

    let mut md5v6 = tcp(true, 50111, 443, 10_000, 20_000, 0x18, 128, &hello, Addr::default());
    let (_, out) = run(&mut proc_with(presets::MD5_TTL3.config()), &mut md5v6, true, 0);
    let m6 = out[0].ip();
    assert!(out.len() == 2 && m6.ttl == 3 && m6.l4_header_len() == 40 && ip_len_matches(&out[0].data, true));

    let mut zero = proc_with(presets::ZERO_FAKE.config());
    let mut p2 = out_tcp(false, 50112, 443, 30_000, &hello);
    let (_, out) = run(&mut zero, &mut p2, true, 0);
    let z = out[0].ip();
    assert!(out.len() == 2 && z.payload_len == 4 && out[0].payload().iter().all(|&b| b == 0) && z.ttl == 5);

    let http = format!("GET / HTTP/1.1\r\nHost: {BLOCKED}\r\n\r\n");
    let mut ph = out_tcp(false, 50113, 80, 1, http.as_bytes());
    let (_, out) = run(&mut zero, &mut ph, true, 0);
    assert!(!contains(&out[0].data, b"Host:") && out[0].ip().payload_len == 4);

    let mut sf = proc_with(presets::SPLIT_FAKE_TTL5.config());
    let mut p3 = out_tcp(false, 50114, 443, 40_000, &hello);
    let (_, out) = run(&mut sf, &mut p3, true, 0);
    assert!(out.len() == 3 && out[2].original);
    let (a, b) = (out[0].ip(), out[1].ip());
    assert!(a.tcp_seq == 40_000 && a.payload_len == 2 && b.tcp_seq == 40_002);
    assert_eq!(b.payload_len, fakes::tls_client_hello().len() - 2);
    assert!(a.ttl == 5 && b.ttl == 5);
    assert_eq!(reassemble(&out[..2], 40_000), fakes::tls_client_hello());

    let mut rep = proc_with(DpiConfig { auto_ttl: false, split_tls: false, fake_repeats: 3, ..DpiConfig::default() });
    let mut p4 = out_tcp(false, 50115, 443, 1, &hello);
    let (_, out) = run(&mut rep, &mut p4, true, 0);
    assert!(out.len() == 4 && out[..3].iter().all(|o| contains(&o.data, FAKE_HOST.as_bytes())));

    let mut clamp = proc_with(DpiConfig { auto_ttl: false, split_tls: false, fake_repeats: 99, ..DpiConfig::default() });
    assert_eq!(run(&mut clamp, &mut p4, true, 0).1.len(), 11, "tekrar 99 -> 10");

    let mut t4 = proc_with(presets::FAKE_TTL4.config());
    let mut p5 = out_tcp(false, 50116, 443, 1, &hello);
    let (_, out) = run(&mut t4, &mut p5, true, 0);
    assert!(out.len() == 2 && out[0].ip().ttl == 4 && out[1].original);
}

fn continuation(v6: bool) {
    let hello = fakes::build_client_hello(BLOCKED, 1800, true);
    const MSS: usize = 1440;
    let sni_at = hello.windows(BLOCKED.len()).position(|w| w == BLOCKED.as_bytes()).unwrap();
    assert!(sni_at > MSS, "test verisi: SNI ikinci pakette");

    let mut proc = default_proc();
    let mut s1 = tcp(v6, 50200, 443, 400_000, 7, 0x10, 128, &hello[..MSS], Addr::default());
    let mut s2 = tcp(v6, 50200, 443, 400_000 + MSS as u32, 7, 0x18, 128, &hello[MSS..], Addr::default());

    let (v, first) = run(&mut proc, &mut s1, true, 0);
    assert_eq!(v, Verdict::Replace);
    assert!(first.iter().any(|o| contains(&o.data, FAKE_HOST.as_bytes())));
    let (v, second) = run(&mut proc, &mut s2, true, 5);
    assert_eq!(v, Verdict::Replace);
    assert_eq!(second.len(), 2);
    assert!(second.iter().all(|o| o.data.len() <= s2.len()));

    let mut sent = first;
    sent.extend(second);
    let real: Vec<Sent> = sent.into_iter().filter(|o| !contains(&o.data, FAKE_HOST.as_bytes())).collect();
    assert!(real.iter().all(|o| !contains(&o.data, BLOCKED.as_bytes())));
    assert_eq!(receiver_view(&real, 400_000), hello);
    let st = proc.stats();
    assert_eq!((st.hello_continuations, st.continuation_splits, st.hellos_without_sni), (1, 1, 1));

    assert_eq!(run(&mut proc, &mut s2, true, 10).0, Verdict::Pass, "tekrar gelen devam paketi");

    let mut p2 = default_proc();
    run(&mut p2, &mut s1, true, 0);
    let mut wrong = tcp(v6, 50200, 443, 400_000 + MSS as u32 + 100, 7, 0x18, 128, &hello[MSS..], Addr::default());
    assert_eq!(run(&mut p2, &mut wrong, true, 5).0, Verdict::Pass);

    let mut p3 = default_proc();
    run(&mut p3, &mut s1, true, 0);
    assert_eq!(run(&mut p3, &mut s2, true, 60_000).0, Verdict::Pass, "eski kayit");

    let early = fakes::build_client_hello(BLOCKED, 1800, false);
    let mut p4 = default_proc();
    let mut e1 = tcp(v6, 50201, 443, 1, 7, 0x10, 128, &early[..MSS], Addr::default());
    let mut e2 = tcp(v6, 50201, 443, 1 + MSS as u32, 7, 0x18, 128, &early[MSS..], Addr::default());
    run(&mut p4, &mut e1, true, 0);
    assert_eq!(run(&mut p4, &mut e2, true, 5).0, Verdict::Pass, "SNI ilk pakette");

    let big = fakes::build_client_hello(BLOCKED, 3000, true);
    let mut p5 = default_proc();
    let mut b1 = tcp(v6, 50202, 443, 5_000, 7, 0x10, 128, &big[..1200], Addr::default());
    let mut b2 = tcp(v6, 50202, 443, 6_200, 7, 0x10, 128, &big[1200..2400], Addr::default());
    let mut b3 = tcp(v6, 50202, 443, 7_400, 7, 0x18, 128, &big[2400..], Addr::default());
    let (_, o1) = run(&mut p5, &mut b1, true, 0);
    let mut big_sent: Vec<Sent> = o1.into_iter().filter(|o| !contains(&o.data, FAKE_HOST.as_bytes())).collect();
    assert_eq!(run(&mut p5, &mut b2, true, 1).0, Verdict::Pass, "orta paket");
    big_sent.push(Sent { data: b2.clone(), recalc: false, corrupt: false, original: true });
    let (v, o3) = run(&mut p5, &mut b3, true, 2);
    assert!(v == Verdict::Replace && o3.len() == 2);
    big_sent.extend(o3);
    assert_eq!(receiver_view(&big_sent, 5_000), big);
    assert!(big_sent.iter().all(|o| !contains(&o.data, BLOCKED.as_bytes())));

    let mut dis = proc_with(presets::DISORDER.config());
    run(&mut dis, &mut s1, true, 0);
    assert_eq!(run(&mut dis, &mut s2, true, 5).0, Verdict::Pass, "ters sira: devam izlenmez");
}

#[test]
fn continuation_v4() {
    continuation(false);
}

#[test]
fn continuation_v6() {
    continuation(true);
}

fn voice(v6: bool) {
    let mut proc = Processor::new(presets::DEFAULT.config(), yandex(), Scope::all());
    let mut disc = udp(v6, 50300, 50004, &discord_discovery(0xBEEF), Addr::default());
    let original = disc.clone();
    let (v, out) = run(&mut proc, &mut disc, true, 0);
    assert_eq!(v, Verdict::Replace);
    assert_eq!(out.len(), 7);
    for f in &out[..6] {
        let ip = f.ip();
        assert!(ip.payload_len == 64 && f.payload().iter().all(|&b| b == 0));
        assert!(ip.src_port == 50300 && ip.dst_port == 50004);
        assert_eq!(u16::from_be_bytes([f.data[ip.l4_offset + 4], f.data[ip.l4_offset + 5]]), 72);
        assert!(ip_len_matches(&f.data, v6));
        assert_eq!(ip.ttl, 64);
    }
    assert!(out[6].original && !out[6].recalc && disc == original);

    let mut stun = udp(v6, 50301, 19302, &stun_binding(0), Addr::default());
    let (v, out) = run(&mut proc, &mut stun, true, 0);
    assert!(v == Verdict::Replace && out.len() == 7);
    let mut stun2 = udp(v6, 50302, 3478, &stun_binding(8), Addr::default());
    assert_eq!(run(&mut proc, &mut stun2, true, 0).0, Verdict::Replace);

    let mut bad = stun_binding(0);
    bad[3] = 4;
    let mut bl = udp(v6, 50303, 3478, &bad, Addr::default());
    assert_eq!(run(&mut proc, &mut bl, true, 0).0, Verdict::Pass);

    let mut rtp = vec![0u8; 74];
    rtp[0] = 0x80;
    rtp[1] = 0x78;
    let mut rp = udp(v6, 50300, 50004, &rtp, Addr::default());
    assert_eq!(run(&mut proc, &mut rp, true, 0).0, Verdict::Pass);

    let mut filled = discord_discovery(1);
    filled[20] = 0x31;
    let mut fp = udp(v6, 50300, 50004, &filled, Addr::default());
    assert_eq!(run(&mut proc, &mut fp, true, 0).0, Verdict::Pass);

    assert_eq!(run(&mut proc, &mut stun, false, 0).0, Verdict::Pass, "gelen STUN");

    let mut off = proc_with(DpiConfig { voice_fake: false, ..DpiConfig::default() });
    assert_eq!(run(&mut off, &mut disc, true, 0).0, Verdict::Pass);
    let mut two = proc_with(DpiConfig { voice_fake_repeats: 2, ..DpiConfig::default() });
    assert_eq!(run(&mut two, &mut disc, true, 0).1.len(), 3);

    let mut query = vec![0u8; 29];
    query[2] = 0x01;
    let mut q = udp(v6, 53010, 53, &query, Addr { dst_last: 1, ..Addr::default() });
    assert_eq!(run(&mut proc, &mut q, true, 0).0, Verdict::Modified);
    assert_eq!(engine::IpPacket::parse(&q).unwrap().dst_port, 1253);

    let st = proc.stats();
    assert_eq!((st.voice_discoveries, st.stun_messages, st.voice_fakes_sent), (1, 2, 18));
}

#[test]
fn voice_v4() {
    voice(false);
}

#[test]
fn voice_v6() {
    voice(true);
}

#[test]
fn http() {
    let mut proc = default_proc();
    let req = format!("GET / HTTP/1.1\r\nHost: {BLOCKED}\r\n\r\n");
    let mut p = out_tcp(false, 50020, 80, 10, req.as_bytes());
    let (v, out) = run(&mut proc, &mut p, true, 0);
    assert_eq!(v, Verdict::Replace);
    assert!(contains(&out[0].data, b"Host: www.w3.org"));
    assert_eq!(reassemble(&out[1..], 10), req.as_bytes());

    let mut no = proc_with(DpiConfig { fragment_http: false, ..DpiConfig::default() });
    assert_eq!(run(&mut no, &mut p, true, 0).0, Verdict::Pass);
    assert!(!no.build_filter().contains("DstPort == 80"));
}

#[test]
fn quic() {
    let mut proc = default_proc();
    let mut initial = vec![0u8; 1250];
    initial[0] = 0xC3;
    initial[1..5].copy_from_slice(&1u32.to_be_bytes());
    let mut pkt = udp(false, 50030, 443, &initial, Addr::default());
    assert_eq!(run(&mut proc, &mut pkt, true, 0).0, Verdict::Drop);

    let mut short = vec![0u8; 1250];
    short[0] = 0x43;
    let mut sh = udp(false, 50030, 443, &short, Addr::default());
    assert_eq!(run(&mut proc, &mut sh, true, 0).0, Verdict::Pass);

    let mut small = udp(false, 50030, 443, &[0xC3, 0, 0, 0, 1, 0, 0], Addr::default());
    assert_eq!(run(&mut proc, &mut small, true, 0).0, Verdict::Pass);

    let mut off = proc_with(DpiConfig { block_quic: false, ..DpiConfig::default() });
    assert_eq!(run(&mut off, &mut pkt, true, 0).0, Verdict::Pass);
}

fn dns(v6: bool) {
    let mut proc = Processor::new(presets::DEFAULT.config(), yandex(), Scope::all());
    let mut query = vec![0u8; 29];
    query[0..2].copy_from_slice(&0xBEEFu16.to_be_bytes());
    query[2] = 0x01;

    let mut q = udp(v6, 53001, 53, &query, Addr { dst_last: 1, ..Addr::default() });
    let orig_dst = engine::IpPacket::parse(&q).unwrap().dst_addr(&q).to_vec();
    assert_eq!(run(&mut proc, &mut q, true, 0).0, Verdict::Modified);
    let qip = engine::IpPacket::parse(&q).unwrap();
    let resolver: Vec<u8> = if v6 { YANDEX_V6.to_vec() } else { vec![77, 88, 8, 8] };
    assert_eq!(qip.dst_addr(&q), &resolver[..]);
    assert_eq!(qip.dst_port, 1253);

    let mut answer = query.clone();
    answer[2] = 0x81;
    let mut resp = udp(v6, 1253, 53001, &answer, Addr { src: Some(resolver.clone()), ..Addr::default() });
    assert_eq!(run(&mut proc, &mut resp, false, 10).0, Verdict::Modified);
    let rip = engine::IpPacket::parse(&resp).unwrap();
    assert_eq!(rip.src_addr(&resp), &orig_dst[..]);
    assert_eq!(rip.src_port, 53);

    let mut resp2 = udp(v6, 1253, 53001, &answer, Addr { src: Some(resolver.clone()), ..Addr::default() });
    assert_eq!(run(&mut proc, &mut resp2, false, 20).0, Verdict::Modified, "ikinci yanit");

    let mut stray = answer.clone();
    stray[0] = 0x12;
    let mut sp = udp(v6, 1253, 53001, &stray, Addr { src: Some(resolver.clone()), ..Addr::default() });
    assert_eq!(run(&mut proc, &mut sp, false, 30).0, Verdict::Pass);

    let cf_v4 = ([1, 1, 1, 1], 53);
    let cf_v6 = ([0x26, 0x06, 0x47, 0, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11], 53);
    let mut cf = Processor::new(
        presets::DEFAULT.config(),
        DnsMode::Redirect(Redirect { v4: Some(cf_v4), v6: Some(cf_v6) }),
        Scope::all(),
    );
    let cf_addr: Vec<u8> = if v6 { cf_v6.0.to_vec() } else { cf_v4.0.to_vec() };
    let mut direct = udp(v6, 53002, 53, &query, Addr { dst: Some(cf_addr), ..Addr::default() });
    assert_eq!(run(&mut cf, &mut direct, true, 0).0, Verdict::Pass, "dogrudan sorgu");

    let mut off = default_proc();
    let mut q2 = udp(v6, 53003, 53, &query, Addr { dst_last: 1, ..Addr::default() });
    assert_eq!(run(&mut off, &mut q2, true, 0).0, Verdict::Pass);

    let mut icp = Processor::new(presets::DEFAULT.config(), DnsMode::Intercept, Scope::all());
    let mut q3 = udp(v6, 53004, 53, &query, Addr { dst_last: 1, ..Addr::default() });
    assert_eq!(run(&mut icp, &mut q3, true, 0).0, Verdict::DnsQuery);
    let mut a3 = udp(v6, 53, 53004, &answer, Addr { dst_last: 1, ..Addr::default() });
    assert_eq!(run(&mut icp, &mut a3, true, 0).0, Verdict::Pass, "yanit yakalanmaz");
}

#[test]
fn dns_v4() {
    dns(false);
}

#[test]
fn dns_v6() {
    dns(true);
}

#[test]
fn config_json_compat_with_v1() {
    // v1'in yazdigi ozel profil (fakePayload sayi olarak).
    let v1 = r#"{"fakePacket":true,"fakeTtl":false,"autoTtl":true,"ttl":4,"fakeWrongChecksum":false,
        "fakeWrongSeq":false,"fakeMd5Sig":true,"fakePayload":1,"splitFake":false,"fakeRepeats":2,
        "splitTls":false,"reverseSplit":true,"splitPosition":2,"splitSni":true,"seqOverlap":0,
        "blockQuic":true,"fragmentHttp":true,"voiceFake":true,"voiceFakeRepeats":6}"#;
    let c: DpiConfig = serde_json::from_str(v1).unwrap();
    assert!(c.fake_md5sig && !c.fake_ttl && c.ttl == 4 && c.fake_repeats == 2);
    assert_eq!(c.fake_payload, engine::FakePayload::Zeros);
    let back: DpiConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(back, c);
    let partial: DpiConfig = serde_json::from_str(r#"{"ttl":3}"#).unwrap();
    assert_eq!(partial, DpiConfig { ttl: 3, ..DpiConfig::default() });
}

#[test]
fn summaries() {
    assert_eq!(presets::DEFAULT.config().summary(), "sahte paket (oto TTL) · ters sıra bölme · QUIC engeli · Discord ses");
    assert_eq!(presets::DISORDER.config().summary(), "ters sıra bölme +1 örtüşme · QUIC engeli · Discord ses");
    assert_eq!(presets::isp("turktelekom").recommended().id, "disorder");
    assert_eq!(presets::isp("bilinmeyen").id, "general");
    for isp in presets::ISPS {
        for m in isp.methods {
            assert!(presets::method(m).is_some(), "{} icinde bilinmeyen yontem {m}", isp.id);
        }
    }
}

#[test]
fn extra_tls_ports() {
    let mut proc = proc_with(DpiConfig { extra_tls_ports: vec![2053, 8443], ..DpiConfig::default() });
    let hello = fakes::build_client_hello(BLOCKED, 700, false);
    let mut p = out_tcp(false, 50400, 8443, 1, &hello);
    assert_eq!(run(&mut proc, &mut p, true, 0).0, Verdict::Replace);
    let mut q = out_tcp(false, 50401, 9000, 1, &hello);
    assert_eq!(run(&mut proc, &mut q, true, 0).0, Verdict::Pass);
    assert!(proc.build_filter().contains("tcp.DstPort == 8443"));
}
