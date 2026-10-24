//! Dil secimi. Arayuz, tepsi ve motor hizmeti ayni kurali kullanir: kaynak metinler Turkce,
//! her metnin Ingilizcesi yaninda yazilir (`t!`). Slint ekranlari `@tr` ile isaretlidir ve
//! Ingilizceleri `apps/ui/lang/en` altindaki `.po` dosyasindan gelir.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Language {
    /// Sistemin (Windows / macOS) arayuz dili Turkce ise Turkce, degilse Ingilizce.
    #[default]
    System,
    Turkish,
    English,
}

impl Language {
    pub fn is_english(self) -> bool {
        match self {
            Self::Turkish => false,
            Self::English => true,
            Self::System => !system_is_turkish(),
        }
    }
}

#[cfg(windows)]
fn system_is_turkish() -> bool {
    use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
    const LANG_TURKISH: u16 = 0x1F;
    (unsafe { GetUserDefaultUILanguage() } & 0x3FF) == LANG_TURKISH
}

/// macOS: tercih edilen dillerin ilki (`AppleLanguages`, orn. "tr-TR"). Surec basina bir kez okunur.
#[cfg(target_os = "macos")]
fn system_is_turkish() -> bool {
    static TURKISH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TURKISH.get_or_init(|| {
        let out = std::process::Command::new("/usr/bin/defaults").args(["read", "-g", "AppleLanguages"]).output();
        match out {
            Ok(o) if o.status.success() => first_language_is_turkish(&String::from_utf8_lossy(&o.stdout)),
            _ => std::env::var("LANG").is_ok_and(|l| l.starts_with("tr")),
        }
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system_is_turkish() -> bool {
    std::env::var("LANG").is_ok_and(|l| l.starts_with("tr"))
}

/// `defaults read -g AppleLanguages` ciktisinin ("(\n    \"tr-TR\",\n ...)") ilk dili Turkce mi?
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn first_language_is_turkish(plist: &str) -> bool {
    plist
        .split(|c: char| c == '(' || c == ',' || c == '\n')
        .map(|s| s.trim().trim_matches('"'))
        .find(|s| !s.is_empty())
        .is_some_and(|l| l == "tr" || l.starts_with("tr-") || l.starts_with("tr_"))
}

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// Surecin metin dilini secer (secilene kadar Turkce).
pub fn set(language: Language) {
    ENGLISH.store(language.is_english(), Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Secili dildeki metin: `t!("Türkçe {x}", "English {x}")`. Bicim argumanlari ikisinde de gecerlidir.
#[macro_export]
macro_rules! t {
    ($tr:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::english() { format!($en $(, $arg)*) } else { format!($tr $(, $arg)*) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_language_and_formats_both() {
        let n = 3;
        set(Language::English);
        assert_eq!(crate::t!("{n} site", "{n} sites"), "3 sites");
        assert_eq!(crate::t!("Kapalı", "Off"), "Off");
        set(Language::Turkish);
        assert_eq!(crate::t!("{n} site", "{n} sites"), "3 site");
        assert_eq!(crate::t!("{} · {}", "{} · {}", "a", n), "a · 3");
        assert!(!english());
    }

    #[test]
    fn reads_the_first_mac_language() {
        assert!(first_language_is_turkish("(\n    \"tr-TR\",\n    \"en-US\"\n)\n"));
        assert!(first_language_is_turkish("(\n    tr\n)"));
        assert!(!first_language_is_turkish("(\n    \"en-TR\",\n    \"tr-TR\"\n)"));
        assert!(!first_language_is_turkish("(\n)"));
    }

    #[test]
    fn language_json_is_stable() {
        assert_eq!(serde_json::to_string(&Language::English).unwrap(), "\"english\"");
        assert_eq!(serde_json::from_str::<Language>("\"system\"").unwrap(), Language::System);
    }
}
