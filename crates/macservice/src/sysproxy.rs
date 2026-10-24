//! Sistem proxy ayari: macOS'ta `networksetup` ile her etkin ag hizmetinin "otomatik proxy
//! yapilandirmasi" PAC adresimize yonlenir. Onceki ayar diske yazilir; baglanti kesilince ya
//! da uygulama cokup yeniden acilinca geri yuklenir.
//!
//! GitHub'in Mac makinelerinde (yonetici kullanici) networksetup sudo istemeden calisti.
//! Standart kullanicida "admin privileges" hatasi gelirse ayni komutlar macOS'un yonetici
//! sifresi penceresiyle (osascript) bir kez daha denenir.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub trait SystemProxy: Send {
    /// Sistem bu PAC adresini kullansin. Ilk cagrida onceki ayar saklanir.
    fn enable(&mut self, pac_url: &str) -> Result<(), String>;
    /// Onceki ayari geri yukler. Etkin degilse bir sey yapmaz.
    fn disable(&mut self) -> Result<(), String>;
}

/// Sistem ayarina dokunmaz (testler ve Mac disi).
pub struct Untouched;

impl SystemProxy for Untouched {
    fn enable(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn disable(&mut self) -> Result<(), String> {
        Ok(())
    }
}

/// Bir ag hizmetinin onceki otomatik proxy ayari.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    pub service: String,
    pub url: String,
    pub enabled: bool,
}

/// `networksetup -listallnetworkservices`: ilk satir aciklama, `*` ile baslayanlar kapali.
pub fn parse_services(out: &str) -> Vec<String> {
    out.lines()
        .skip(1)
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with('*'))
        .map(str::to_owned)
        .collect()
}

/// `networksetup -getautoproxyurl X`: "URL: ...\nEnabled: Yes|No".
pub fn parse_autoproxy(out: &str) -> (String, bool) {
    let mut url = String::new();
    let mut enabled = false;
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("URL:") {
            let v = v.trim();
            if v != "(null)" {
                url = v.to_owned();
            }
        } else if let Some(v) = line.strip_prefix("Enabled:") {
            enabled = v.trim().eq_ignore_ascii_case("yes");
        }
    }
    (url, enabled)
}

/// Bizim PAC adresimiz mi? (Cokmeden kalan eski ayar geri "yuklenmez", kapatilir.)
pub fn is_ours(url: &str) -> bool {
    url.starts_with("http://127.0.0.1:") && url.contains(proxy::PAC_PATH)
}

/// Geri yukleme komutlari (networksetup argumanlari).
pub fn restore_commands(saved: &[Saved]) -> Vec<Vec<String>> {
    saved
        .iter()
        .map(|s| {
            if s.enabled && !s.url.is_empty() && !is_ours(&s.url) {
                vec!["-setautoproxyurl".into(), s.service.clone(), s.url.clone()]
            } else {
                vec!["-setautoproxystate".into(), s.service.clone(), "off".into()]
            }
        })
        .collect()
}

/// Kabuk icin tek tirnakli arguman.
pub fn sh_quote(a: &str) -> String {
    format!("'{}'", a.replace('\'', r"'\''"))
}

/// AppleScript dizgisi icin kacis.
pub fn applescript_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', "\\\"")
}

/// Yonetici penceresiyle calistirilacak tek kabuk satiri.
pub fn admin_script(cmds: &[Vec<String>]) -> String {
    let shell: Vec<String> = cmds
        .iter()
        .map(|c| std::iter::once(NETWORKSETUP.to_owned()).chain(c.iter().map(|a| sh_quote(a))).collect::<Vec<_>>().join(" "))
        .collect();
    format!("do shell script \"{}\" with administrator privileges", applescript_escape(&shell.join(" && ")))
}

const NETWORKSETUP: &str = "/usr/sbin/networksetup";

/// macOS uygulamasi: `networksetup`.
pub struct Networksetup {
    state_file: PathBuf,
    saved: Option<Vec<Saved>>,
}

impl Networksetup {
    pub fn new(state_file: PathBuf) -> Self {
        Self { state_file, saved: None }
    }

    /// Onceki calisma sistem ayarini geri almadan bittiyse simdi geri alir.
    pub fn restore_stale(&mut self) -> Option<String> {
        let text = std::fs::read_to_string(&self.state_file).ok()?;
        let saved: Vec<Saved> = serde_json::from_str(&text).ok()?;
        self.saved = Some(saved);
        Some(match self.disable() {
            Ok(()) => "Önceki çalışmadan kalan sistem proxy ayarı geri alındı.".into(),
            Err(e) => format!("Önceki çalışmadan kalan sistem proxy ayarı geri alınamadı: {e}"),
        })
    }

    fn run(args: &[String]) -> Result<String, String> {
        let out = std::process::Command::new(NETWORKSETUP).args(args).output().map_err(|e| format!("networksetup: {e}"))?;
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        // Hata durumunda da cikis kodu 0 olabiliyor; metne bakilir (hizmet adlarina takilmadan).
        if !out.status.success() || text.contains("** Error") || text.contains("requires admin") {
            return Err(text.trim().to_owned());
        }
        Ok(text)
    }

    fn services() -> Result<Vec<String>, String> {
        let list = parse_services(&Self::run(&["-listallnetworkservices".into()])?);
        if list.is_empty() {
            return Err("etkin ağ hizmeti bulunamadı".into());
        }
        Ok(list)
    }

    /// Komutlari calistirir; yetki yetmezse hepsini yonetici penceresiyle bir kez daha dener.
    fn apply(cmds: &[Vec<String>]) -> Result<(), String> {
        let mut failed = None;
        for c in cmds {
            if let Err(e) = Self::run(c) {
                failed = Some(e);
                break;
            }
        }
        let Some(first) = failed else { return Ok(()) };
        let out = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &admin_script(cmds)])
            .output()
            .map_err(|e| format!("osascript: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("{first} / {}", String::from_utf8_lossy(&out.stderr).trim()))
        }
    }
}

impl SystemProxy for Networksetup {
    fn enable(&mut self, pac_url: &str) -> Result<(), String> {
        let services = Self::services()?;
        if self.saved.is_none() {
            let mut saved = Vec::with_capacity(services.len());
            for s in &services {
                let (url, enabled) = parse_autoproxy(&Self::run(&["-getautoproxyurl".into(), s.clone()])?);
                saved.push(Saved { service: s.clone(), url, enabled });
            }
            // Once diske: ayari degistirip cokersek bir sonraki acilis geri alabilsin.
            let json = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?;
            std::fs::write(&self.state_file, json).map_err(|e| format!("{}: {e}", self.state_file.display()))?;
            self.saved = Some(saved);
        }
        let cmds: Vec<Vec<String>> = services.iter().map(|s| vec!["-setautoproxyurl".into(), s.clone(), pac_url.to_owned()]).collect();
        Self::apply(&cmds)
    }

    fn disable(&mut self) -> Result<(), String> {
        let Some(saved) = self.saved.take() else { return Ok(()) };
        let result = Self::apply(&restore_commands(&saved));
        if result.is_ok() {
            let _ = std::fs::remove_file(&self.state_file);
        } else {
            self.saved = Some(saved);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_networksetup_output() {
        let list = "An asterisk (*) denotes that a network service is disabled.\nWi-Fi\nThunderbolt Bridge\n*Bluetooth PAN\nUSB 10/100/1000 LAN\n";
        assert_eq!(parse_services(list), ["Wi-Fi", "Thunderbolt Bridge", "USB 10/100/1000 LAN"]);
        assert_eq!(parse_autoproxy("URL: (null)\nEnabled: No\n"), (String::new(), false));
        assert_eq!(parse_autoproxy("URL: http://wpad/x.pac\nEnabled: Yes\n"), ("http://wpad/x.pac".into(), true));
    }

    #[test]
    fn restores_previous_settings_but_never_our_own() {
        let saved = vec![
            Saved { service: "Wi-Fi".into(), url: "http://wpad/x.pac".into(), enabled: true },
            Saved { service: "Ethernet".into(), url: String::new(), enabled: false },
            Saved { service: "USB".into(), url: "http://127.0.0.1:47813/proxy.pac?v=3".into(), enabled: true },
            Saved { service: "Kapali".into(), url: "http://wpad/y.pac".into(), enabled: false },
        ];
        let cmds = restore_commands(&saved);
        assert_eq!(cmds[0], ["-setautoproxyurl", "Wi-Fi", "http://wpad/x.pac"]);
        assert_eq!(cmds[1], ["-setautoproxystate", "Ethernet", "off"]);
        assert_eq!(cmds[2], ["-setautoproxystate", "USB", "off"], "cokmeden kalan kendi ayarimiz");
        assert_eq!(cmds[3], ["-setautoproxystate", "Kapali", "off"]);
    }

    #[test]
    fn admin_script_quotes_service_names() {
        let s = admin_script(&[vec!["-setautoproxyurl".into(), "Ali's \"Wi-Fi\"".into(), "http://127.0.0.1:1/proxy.pac".into()]]);
        assert_eq!(
            s,
            r#"do shell script "/usr/sbin/networksetup '-setautoproxyurl' 'Ali'\\''s \"Wi-Fi\"' 'http://127.0.0.1:1/proxy.pac'" with administrator privileges"#
        );
    }
}
