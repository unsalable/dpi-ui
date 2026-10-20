//! Gelistirmede WinDivert dosyalarini derlenen exe'nin yanina kopyalar
//! (kurulumda bunu Inno Setup yapar).

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("../../third_party/windivert/2.2");
    println!("cargo:rerun-if-changed={}", src.display());

    // OUT_DIR = target/<profil>/build/<paket>-<hash>/out -> uc ust klasor exe'nin yeri.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let Some(target) = out.ancestors().nth(3) else { return };
    for name in ["WinDivert.dll", "WinDivert64.sys"] {
        copy_if_changed(&src.join(name), &target.join(name));
    }
}

fn copy_if_changed(from: &Path, to: &Path) {
    let Ok(data) = std::fs::read(from) else { return };
    if std::fs::read(to).ok().as_deref() != Some(&data[..]) {
        let _ = std::fs::write(to, data);
    }
}
