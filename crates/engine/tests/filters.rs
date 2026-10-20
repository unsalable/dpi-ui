//! Uretilen her WinDivert filtresi gercek WinDivert derleyicisinden gecmeli
//! (WinDivertHelperCompileFilter; surucu ve yonetici gerekmez).

use std::path::Path;

use engine::{DnsMode, DpiConfig, HostSet, Processor, Redirect, Scope, ScopeMode, presets};

#[test]
fn every_generated_filter_compiles() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/windivert/2.2");
    windivert::load(&dir).expect("WinDivert.dll yuklenemedi");

    let dns_modes = [
        DnsMode::Off,
        DnsMode::Intercept,
        DnsMode::Redirect(Redirect {
            v4: Some(([77, 88, 8, 8], 1253)),
            v6: Some(([0x2a, 0x02, 0x06, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe, 0xed, 0x00, 0xff], 1253)),
        }),
        DnsMode::Redirect(Redirect { v4: Some(([1, 1, 1, 1], 53)), v6: None }),
    ];
    let scopes = [
        Scope::all(),
        Scope { mode: ScopeMode::Smart, targets: HostSet::new(["discord.com"]), learning: true, ..Scope::default() },
    ];

    let mut configs: Vec<DpiConfig> = presets::ALL.iter().map(|m| m.config()).collect();
    configs.push(DpiConfig { extra_tls_ports: vec![2053, 2083, 2087, 2096, 8443], ..DpiConfig::default() });
    configs.push(DpiConfig {
        fake_packet: false,
        split_tls: false,
        block_quic: false,
        fragment_http: false,
        voice_fake: false,
        ..DpiConfig::default()
    });

    let mut checked = 0;
    for cfg in &configs {
        for dns in dns_modes {
            for scope in &scopes {
                let f = Processor::new(cfg.clone(), dns, scope.clone()).build_filter();
                if let Err(e) = windivert::compile_filter(&f, windivert::Layer::Network) {
                    panic!("filtre derlenmedi: {e}\n{f}");
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, configs.len() * dns_modes.len() * scopes.len());
}
