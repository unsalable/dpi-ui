//! Arayuzun VPN olup olmadigini anlar (sonuc 60 sn onbellekte). VPN uzerinden giden
//! DNS sorgulari VPN'in kendi DNS'ine birakilir: kurum ici adlar bozulmasin.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows_sys::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};

const TTL: Duration = Duration::from_secs(60);
const IF_TYPE_PPP: u32 = 23;
const IF_TYPE_PROP_VIRTUAL: u32 = 53;
const IF_TYPE_TUNNEL: u32 = 131;
const KEYWORDS: &[&str] = &["vpn", "wireguard", "wintun", "openvpn", "tap-windows", "tap adapter", "fortinet", "cisco anyconnect", "zerotier", "tailscale", "warp"];

#[derive(Default)]
pub struct VpnDetector {
    cache: Mutex<HashMap<u32, (bool, Instant)>>,
}

impl VpnDetector {
    pub fn is_vpn(&self, if_idx: u32) -> bool {
        if if_idx == 0 {
            return false;
        }
        let now = Instant::now();
        if let Some((v, at)) = self.cache.lock().unwrap().get(&if_idx) {
            if now.duration_since(*at) < TTL {
                return *v;
            }
        }
        let v = query(if_idx);
        let mut cache = self.cache.lock().unwrap();
        if cache.len() > 64 {
            cache.clear();
        }
        cache.insert(if_idx, (v, now));
        v
    }
}

fn query(if_idx: u32) -> bool {
    unsafe {
        let mut row: MIB_IF_ROW2 = std::mem::zeroed();
        row.InterfaceIndex = if_idx;
        if GetIfEntry2(&mut row) != 0 {
            return false;
        }
        if matches!(row.Type, IF_TYPE_PPP | IF_TYPE_PROP_VIRTUAL | IF_TYPE_TUNNEL) || row.TunnelType != 0 {
            return true;
        }
        let len = row.Description.iter().position(|&c| c == 0).unwrap_or(row.Description.len());
        let desc = String::from_utf16_lossy(&row.Description[..len]).to_lowercase();
        KEYWORDS.iter().any(|k| desc.contains(k))
    }
}
