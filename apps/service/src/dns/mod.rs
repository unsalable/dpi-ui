//! Sifreli DNS: yakalanan sorgulari DoH ile cozup yaniti enjekte eder.

pub mod bench;
pub mod cache;
pub mod doh;
pub mod message;
pub mod resolver;
pub mod vpn;

pub use resolver::{DnsJob, DnsStats, Options, Resolver};
