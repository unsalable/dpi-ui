//! Telefon penceresinin Slint derlemesi. Sayfalar ve ceviriler masaustu arayuzuyle ortak
//! (`apps/ui/ui`, `apps/ui/lang`); yalnizca kok bilesen `mobile.slint`.

use std::path::PathBuf;

/// Masaustu arayuzunun ceviri alani (paket adi).
const DOMAIN: &str = "goodbyedpi3-ui";

fn main() {
    // Slint .po dosyasini bu paketin adiyla arar; ortak dosyalar derleme klasorune o adla kopyalanir.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("lang");
    let pkg = std::env::var("CARGO_PKG_NAME").unwrap();
    for entry in std::fs::read_dir("../ui/lang").expect("ceviri klasoru") {
        let lang = entry.unwrap().path();
        let src = lang.join("LC_MESSAGES").join(format!("{DOMAIN}.po"));
        if src.exists() {
            let dir = out.join(lang.file_name().unwrap()).join("LC_MESSAGES");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::copy(&src, dir.join(format!("{pkg}.po"))).unwrap();
        }
    }
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations(&out)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("../ui/ui/mobile.slint", config).expect("Slint derlemesi basarisiz");
    println!("cargo:rerun-if-changed=../ui/lang");
}
