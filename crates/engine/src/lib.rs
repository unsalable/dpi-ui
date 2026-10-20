//! GoodbyeDPI UI 3 motoru: DPI atlatma paket mantigi.
//!
//! Bu crate isletim sistemine, surucuye ya da ag erisimine dokunmaz; yalnizca
//! paket baytlari uzerinde karar verir. WinDivert dongusu servis tarafindadir.

pub mod config;
pub mod fakes;
pub mod output;
pub mod packet;
pub mod presets;
pub mod processor;
pub mod scope;
pub mod tls;

pub use config::{DpiConfig, FakePayload};
pub use output::{ItemKind, OutItem, Output};
pub use packet::IpPacket;
pub use processor::{DnsMode, Event, LearnReason, Processor, Redirect, Stats, Verdict};
pub use scope::{HostSet, Scope, ScopeMode};
