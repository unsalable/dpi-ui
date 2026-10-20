//! Slint derlemesi + Windows kaynaklari (kodla cizilen simge, bildirim dosyasi, surum bilgisi).

#[path = "src/iconart.rs"]
#[allow(dead_code)]
mod iconart;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="GoodbyeDPI3" version="3.0.0.0" processorArchitecture="amd64"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
      <heapType xmlns="http://schemas.microsoft.com/SMI/2020/WindowsSettings">SegmentHeap</heapType>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

fn main() {
    slint_build::compile("ui/app.slint").expect("Slint derlemesi basarisiz");
    println!("cargo:rerun-if-changed=src/iconart.rs");

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("app.ico");
    std::fs::write(&ico, iconart::build_ico(&[16, 20, 24, 32, 40, 48, 64, 256], iconart::Mark::App)).unwrap();

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().unwrap());
    res.set_manifest(MANIFEST);
    res.set("ProductName", "GoodbyeDPI UI 3");
    res.set("FileDescription", "GoodbyeDPI UI 3");
    res.set("CompanyName", "GoodbyeDPI UI");
    res.set("LegalCopyright", "GPL-3.0");
    res.set("OriginalFilename", "GoodbyeDPI3.exe");
    if let Err(e) = res.compile() {
        println!("cargo:warning=Windows kaynaklari derlenemedi: {e}");
    }
}
