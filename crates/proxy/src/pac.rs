//! Otomatik proxy ayar dosyasi (PAC). Mac'te sistem proxy'si bu dosyayi gosterir; boylece
//! "Akilli liste"de yalnizca listedeki siteler proxy'den gecer, gerisi dogrudan gider.
//! Proxy kapaliyken ya da ulasilamazsa tarayici dogrudan baglanir ("; DIRECT").

use std::net::SocketAddr;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PacScope {
    /// Her site proxy'den gecsin (istisnalar haric).
    pub all: bool,
    /// Akilli modda proxy'den gececek alan adlari (alt alan adlari dahil).
    pub targets: Vec<String>,
    /// Hicbir zaman proxy'den gecmeyecek alan adlari.
    pub exclude: Vec<String>,
}

/// Her seyi dogrudan gonderen betik (baglanti kesikken).
pub fn direct() -> String {
    "function FindProxyForURL(url, host) {\n  return \"DIRECT\";\n}\n".to_owned()
}

pub fn script(proxy: SocketAddr, scope: &PacScope) -> String {
    let route = format!("PROXY {proxy}; DIRECT");
    let list = |names: &[String]| {
        let mut items: Vec<String> = names.iter().filter_map(|n| normalize(n)).map(|n| format!("\"{n}\": 1")).collect();
        items.sort();
        items.dedup();
        format!("{{{}}}", items.join(", "))
    };
    format!(
        "var TARGETS = {targets};\n\
         var EXCLUDE = {exclude};\n\
         var ALL = {all};\n\
         function listed(h, set) {{\n\
         \x20 for (;;) {{\n\
         \x20   if (Object.prototype.hasOwnProperty.call(set, h)) return true;\n\
         \x20   var i = h.indexOf(\".\");\n\
         \x20   if (i < 0) return false;\n\
         \x20   h = h.substring(i + 1);\n\
         \x20 }}\n\
         }}\n\
         function FindProxyForURL(url, host) {{\n\
         \x20 host = host.toLowerCase();\n\
         \x20 if (isPlainHostName(host) || host == \"localhost\" || listed(host, EXCLUDE)) return \"DIRECT\";\n\
         \x20 if (/^[0-9.]+$/.test(host) && (isInNet(host, \"10.0.0.0\", \"255.0.0.0\") || isInNet(host, \"172.16.0.0\", \"255.240.0.0\") ||\n\
         \x20     isInNet(host, \"192.168.0.0\", \"255.255.0.0\") || isInNet(host, \"127.0.0.0\", \"255.0.0.0\") ||\n\
         \x20     isInNet(host, \"169.254.0.0\", \"255.255.0.0\"))) return \"DIRECT\";\n\
         \x20 if (host.indexOf(\":\") >= 0) return ALL ? \"{route}\" : \"DIRECT\";\n\
         \x20 if (ALL || listed(host, TARGETS)) return \"{route}\";\n\
         \x20 return \"DIRECT\";\n\
         }}\n",
        targets = list(&scope.targets),
        exclude = list(&scope.exclude),
        all = scope.all,
    )
}

/// Betige yalnizca gecerli ad karakterleri girer (enjeksiyona kapali).
fn normalize(name: &str) -> Option<String> {
    let n = name.trim().trim_start_matches("*.").trim_matches('.').to_ascii_lowercase();
    let ok = !n.is_empty() && n.len() <= 253 && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_');
    ok.then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_valid_names() {
        let s = script(
            "127.0.0.1:47813".parse().unwrap(),
            &PacScope { all: false, targets: vec!["*.Discord.com".into(), "x\"; alert(1); //".into(), "wattpad.com".into()], exclude: vec![] },
        );
        assert!(s.contains("var TARGETS = {\"discord.com\": 1, \"wattpad.com\": 1};"), "{s}");
        assert!(!s.contains("alert"));
        assert!(s.contains("return \"PROXY 127.0.0.1:47813; DIRECT\";"));
        assert!(s.contains("var ALL = false;"));
        assert!(direct().contains("DIRECT") && !direct().contains("PROXY"));
    }
}
