//! Servis ve arayuzun ortak dili.

pub mod config;
pub mod dns;
pub mod ipc;
pub mod migrate;
pub mod paths;
pub mod pipeio;

pub const PRODUCT_NAME: &str = "GoodbyeDPI UI 3";
pub const SERVICE_NAME: &str = "GoodbyeDPI3";
pub const SERVICE_DISPLAY_NAME: &str = "GoodbyeDPI UI 3 Motoru";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
