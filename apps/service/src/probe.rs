//! Canli motor testi (yonetici): engelli bilinen siteleri motor kapali ve acikken
//! curl.exe ile dener, sonucu karsilastirmali yazar. v1'deki `--enginetest --curl`.

use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use engine::{DnsMode, Processor};
use shared::config::ActiveDns;

use crate::engine_loop::{self, DnsSetup, EngineMsg};

const TARGETS: &[&str] = &[
    "https://discord.com/",
    "https://gateway.discord.gg/",
    "https://cdn.discordapp.com/",
    "https://www.roblox.com/",
    "https://www.wattpad.com/",
    "https://example.com/",
];

struct Outcome {
    code: String,
    seconds: f32,
}

fn curl(url: &str) -> Outcome {
    let out = Command::new("curl.exe")
        .args(["-s", "-o", "NUL", "--max-time", "8", "-w", "%{http_code} %{time_total}", url])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let mut parts = text.split_whitespace();
            let code = parts.next().unwrap_or("000").to_owned();
            let seconds = parts.next().and_then(|s| s.replace(',', ".").parse().ok()).unwrap_or(0.0);
            Outcome { code, seconds }
        }
        Err(_) => Outcome { code: "curl yok".into(), seconds: 0.0 },
    }
}

fn ok(o: &Outcome) -> bool {
    o.code != "000" && !o.code.starts_with("curl")
}

pub fn run(method_id: Option<&str>) -> i32 {
    let mut cfg = crate::store::load_config();
    if let Some(m) = method_id {
        cfg.method = m.to_owned();
    }
    // Test tum trafige uygulanir: akilli listenin ogrenme davranisindan bagimsiz sonuc.
    cfg.scope.mode = engine::ScopeMode::All;
    let method = cfg.active_method();
    let dns = cfg.active_dns();
    println!("Yöntem: {} ({})\nDNS: {}\n", method.name, method.config.summary(false), dns.display_name());

    let conflicts = crate::conflicts::scan();
    for c in &conflicts {
        println!("UYARI: {} (PID {}). Doğru sonuç için önce onu kapat.", c.description, c.pid);
    }

    println!("1/2 Motor kapalı:");
    let before: Vec<Outcome> = TARGETS.iter().map(|u| curl(u)).collect();
    for (u, o) in TARGETS.iter().zip(&before) {
        println!("   {:<32} {:>4}  {:.2} sn", u, o.code, o.seconds);
    }

    let (dns_mode, setup) = match &dns {
        ActiveDns::Encrypted { url, bootstrap, .. } => (
            DnsMode::Intercept,
            Some(DnsSetup {
                url: url.clone(),
                bootstrap: bootstrap.clone(),
                options: crate::dns::Options { fallback_to_system: true, skip_on_vpn: true },
            }),
        ),
        _ => (DnsMode::Off, None),
    };
    let processor = Processor::new(method.config.clone(), dns_mode, cfg.build_scope(Vec::<String>::new()));
    let (tx, rx) = mpsc::channel::<EngineMsg>();
    let dir = shared::paths::exe_dir().unwrap_or_default();
    let engine = match engine_loop::start(&dir, processor, setup, tx) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("\nMotor başlatılamadı: {e}");
            return 1;
        }
    };
    std::thread::sleep(Duration::from_millis(300));

    println!("\n2/2 Motor açık:");
    let after: Vec<Outcome> = TARGETS.iter().map(|u| curl(u)).collect();
    for (u, o) in TARGETS.iter().zip(&after) {
        println!("   {:<32} {:>4}  {:.2} sn", u, o.code, o.seconds);
    }
    let stats = engine.stats();
    engine.stop();
    while rx.try_recv().is_ok() {}

    println!(
        "\nSayaçlar: atlatılan {} · sahte {} · QUIC düşen {} · DNS {} (önbellek {}, hata {})",
        stats.bypassed, stats.fakes_sent, stats.quic_dropped, stats.dns_queries, stats.dns_cached, stats.dns_failures
    );
    let fixed = before.iter().zip(&after).filter(|(b, a)| !ok(b) && ok(a)).count();
    let broken = before.iter().zip(&after).filter(|(b, a)| ok(b) && !ok(a)).count();
    let blocked_after = after.iter().filter(|a| !ok(a)).count();
    println!("Sonuç: {fixed} site açıldı, {broken} site bozuldu, {blocked_after} site hâlâ açılmıyor.");
    if broken > 0 || blocked_after > 0 { 2 } else { 0 }
}
