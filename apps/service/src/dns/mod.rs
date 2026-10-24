//! Sifreli DNS: yakalanan sorgulari DoH ile cozup yaniti enjekte eder.

pub mod bench;
pub mod resolver;
pub mod vpn;

// Istemci, mesaj ve onbellek Mac motoruyla ortak: crates/dnsclient.
pub use dnsclient::{cache, doh, message};
pub use resolver::{DnsJob, DnsStats, Options, Resolver};
