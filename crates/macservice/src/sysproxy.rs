//! Sistem proxy ayari, iki yoldan:
//! 1. `networksetup` ile her etkin ag hizmetinin "otomatik proxy yapilandirmasi" PAC adresimize
//!    yonlenir. Safari, Chrome, Discord gibi sistem ayarina uyan uygulamalar bunu kullanir.
//! 2. Oturumun ortam degiskenlerine (`launchctl setenv`) HTTPS_PROXY/HTTP_PROXY yazilir. Sistem
//!    ayarina bakmayan ama bu degiskenlere uyan uygulamalar (olculdu: Roblox oyunu) bununla
//!    proxy'ye gelir. Proxy yalnizca listedeki sitelere dokunur, gerisini oldugu gibi tasir.
//!    Degiskenler baglandiktan sonra acilan uygulamalara gecer.
//!
//! Onceki ayarlar diske yazilir; baglanti kesilince ya da uygulama cokup yeniden acilinca geri
//! yuklenir. GitHub'in Mac makinelerinde (yonetici kullanici) networksetup sudo istemeden calisti;
//! standart kullanicida "admin privileges" hatasi gelirse ayni komutlar macOS'un yonetici sifresi
//! penceresiyle (osascript) bir kez daha denenir.

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub trait SystemProxy: Send {
    /// Sistem bu PAC adresini, ortam degiskenleri de `proxy`'yi gostersin. Ilk cagrida onceki
    /// ayar saklanir.
    fn enable(&mut self, pac_url: &str, proxy: SocketAddr) -> Result<(), String>;
    /// Onceki ayari geri yukler. Etkin degilse bir sey yapmaz.
    fn disable(&mut self) -> Result<(), String>;
}

/// Sistem ayarina dokunmaz (testler ve Mac disi).
pub struct Untouched;

impl SystemProxy for Untouched {
    fn enable(&mut self, _: &str, _: SocketAddr) -> Result<(), String> {
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

/// Bir ortam degiskeninin onceki degeri.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedEnv {
    pub name: String,
    pub value: Option<String>,
}

/// Diskteki durum (`sysproxy.json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub services: Vec<Saved>,
    #[serde(default)]
    pub env: Vec<SavedEnv>,
}

impl State {
    /// Durum dosyasini okur; 3.1.2-mac.3 ve oncesinin bicimi (yalnizca hizmet listesi) da olur.
    pub fn parse(text: &str) -> Option<State> {
        serde_json::from_str(text).ok().or_else(|| serde_json::from_str::<Vec<Saved>>(text).ok().map(|services| State { services, env: Vec::new() }))
    }
}

/// Uygulamalarin proxy icin okudugu degiskenler (curl ve turevleri hem buyuk hem kucuk harf okur).
pub const PROXY_ENV: [&str; 4] = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"];
pub const NO_PROXY_ENV: [&str; 2] = ["NO_PROXY", "no_proxy"];
pub const NO_PROXY: &str = "localhost,127.0.0.1,::1,.local";

/// Yazilacak degiskenler ve degerleri.
pub fn env_values(proxy: SocketAddr) -> Vec<(&'static str, String)> {
    let url = format!("http://{proxy}");
    PROXY_ENV.iter().map(|n| (*n, url.clone())).chain(NO_PROXY_ENV.iter().map(|n| (*n, NO_PROXY.to_owned()))).collect()
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

/// Bizim proxy degiskenimiz mi? (Cokmeden kalan deger geri yuklenmez, silinir.)
fn is_our_env(value: &str) -> bool {
    value.starts_with("http://127.0.0.1:") || value == NO_PROXY
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

/// Ortam degiskenlerini geri yukleme komutlari (launchctl argumanlari).
pub fn env_restore_commands(saved: &[SavedEnv]) -> Vec<Vec<String>> {
    saved
        .iter()
        .map(|e| match &e.value {
            Some(v) if !v.is_empty() && !is_our_env(v) => vec!["setenv".into(), e.name.clone(), v.clone()],
            _ => vec!["unsetenv".into(), e.name.clone()],
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
const LAUNCHCTL: &str = "/bin/launchctl";

/// macOS uygulamasi: `networksetup` ve `launchctl`.
pub struct Networksetup {
    state_file: PathBuf,
    saved: Option<State>,
}

impl Networksetup {
    pub fn new(state_file: PathBuf) -> Self {
        Self { state_file, saved: None }
    }

    /// Onceki calisma sistem ayarini geri almadan bittiyse simdi geri alir.
    pub fn restore_stale(&mut self) -> Option<String> {
        let text = std::fs::read_to_string(&self.state_file).ok()?;
        self.saved = Some(State::parse(&text)?);
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

    fn launchctl(args: &[String]) -> Result<String, String> {
        let out = std::process::Command::new(LAUNCHCTL).args(args).output().map_err(|e| format!("launchctl: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
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

    fn save(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.saved).map_err(|e| e.to_string())?;
        std::fs::write(&self.state_file, json).map_err(|e| format!("{}: {e}", self.state_file.display()))
    }
}

impl SystemProxy for Networksetup {
    fn enable(&mut self, pac_url: &str, proxy: SocketAddr) -> Result<(), String> {
        let services = Self::services()?;
        if self.saved.is_none() {
            let mut state = State::default();
            for s in &services {
                let (url, enabled) = parse_autoproxy(&Self::run(&["-getautoproxyurl".into(), s.clone()])?);
                state.services.push(Saved { service: s.clone(), url, enabled });
            }
            for (name, _) in env_values(proxy) {
                let value = Self::launchctl(&["getenv".into(), name.into()]).ok().filter(|v| !v.is_empty());
                state.env.push(SavedEnv { name: name.into(), value });
            }
            // Once diske: ayari degistirip cokersek bir sonraki acilis geri alabilsin.
            self.saved = Some(state);
            self.save()?;
        }
        let cmds: Vec<Vec<String>> = services.iter().map(|s| vec!["-setautoproxyurl".into(), s.clone(), pac_url.to_owned()]).collect();
        Self::apply(&cmds)?;
        for (name, value) in env_values(proxy) {
            Self::launchctl(&["setenv".into(), name.into(), value])?;
        }
        Ok(())
    }

    fn disable(&mut self) -> Result<(), String> {
        let Some(saved) = self.saved.take() else { return Ok(()) };
        let mut result = Self::apply(&restore_commands(&saved.services));
        for c in env_restore_commands(&saved.env) {
            if let Err(e) = Self::launchctl(&c) {
                result = result.and(Err(e));
            }
        }
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
    fn env_is_set_for_the_proxy_and_restored() {
        let v = env_values("127.0.0.1:47813".parse().unwrap());
        assert!(v.contains(&("HTTPS_PROXY", "http://127.0.0.1:47813".into())));
        assert!(v.contains(&("http_proxy", "http://127.0.0.1:47813".into())));
        assert!(v.contains(&("no_proxy", NO_PROXY.into())));
        let saved = vec![
            SavedEnv { name: "HTTPS_PROXY".into(), value: Some("http://kurum:8080".into()) },
            SavedEnv { name: "https_proxy".into(), value: None },
            SavedEnv { name: "HTTP_PROXY".into(), value: Some("http://127.0.0.1:47813".into()) },
        ];
        let cmds = env_restore_commands(&saved);
        assert_eq!(cmds[0], ["setenv", "HTTPS_PROXY", "http://kurum:8080"], "kullanicinin kendi degeri geri gelir");
        assert_eq!(cmds[1], ["unsetenv", "https_proxy"]);
        assert_eq!(cmds[2], ["unsetenv", "HTTP_PROXY"], "cokmeden kalan kendi degerimiz silinir");
    }

    #[test]
    fn reads_old_and_new_state_files() {
        let old = r#"[{"service":"Wi-Fi","url":"","enabled":false}]"#;
        let s = State::parse(old).unwrap();
        assert_eq!((s.services.len(), s.env.len()), (1, 0));
        let new = serde_json::to_string(&State { services: vec![], env: vec![SavedEnv { name: "HTTPS_PROXY".into(), value: None }] }).unwrap();
        assert_eq!(State::parse(&new).unwrap().env.len(), 1);
        assert!(State::parse("bozuk").is_none());
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
