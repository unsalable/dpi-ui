//! Servis ve arayuzun ortak dili.

pub mod config;
pub mod dns;
pub mod i18n;
pub mod ipc;
pub mod migrate;
pub mod paths;
pub mod pipeio;
pub mod update;

pub const PRODUCT_NAME: &str = "DPI UI";
pub const SERVICE_NAME: &str = "GoodbyeDPI3";
/// Arayuz (pencere ve tepsi) exe'si; servisle ayni klasordedir.
pub const UI_EXE: &str = "GoodbyeDPI3.exe";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
