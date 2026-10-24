//! DPI UI'nin Mac motoru: yerel proxy.
//!
//! Mac'te WinDivert'in karsiligi yok (Apple'in Network Extension'i ucretli gelistirici
//! hesabi ve ozel izin ister). Bu yuzden Mac surumu, Mac'te calisan diger araclar gibi
//! (SpoofDPI, zapret tpws SOCKS modu) bilgisayarda bir proxy acar; sistem proxy ayari PAC
//! dosyasiyla buna yonlenir, proxy de siteye giden ilk mesaji parcalar. Kod isletim
//! sistemine bagli degil: ayni motor Windows'ta da calisir, boylece yontemler Mac olmadan
//! Turkiye'deki hatlarda denenebilir (`dpi-proxy`).

pub mod desync;
pub mod pac;
pub mod presets;
pub mod resolve;
pub mod server;

pub use desync::{Desync, Pos};
pub use resolve::{Resolve, SystemResolver};
#[cfg(feature = "doh")]
pub use resolve::{DohResolver, UdpResolver};
pub use server::{DEFAULT_PORT, Event, Options, PAC_PATH, Proxy, Stats};
