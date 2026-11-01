//! Android'de trafigi proxy'ye getiren katman (Mac'teki sistem proxy ayarinin karsiligi).
//!
//! Java'daki VPN hizmeti bir arayuz (TUN) kurar; butun uygulamalarin IPv4/IPv6 trafigi oraya
//! gelir. tun2proxy bu paketleri TCP baglantilarina ve UDP akislarina cevirip yerel proxy'ye
//! SOCKS5 ile verir. Uygulamanin kendisi VPN'in disindadir (Java: `addDisallowedApplication`);
//! proxy'nin sitelere actigi baglantilar ve sifreli DNS sorgulari VPN'e geri girmez.
//!
//! Adlar sanal DNS ile cozulur: uygulamaya gecici bir adres (198.18.0.0/15) verilir, proxy'ye
//! adin kendisi gider. Ad proxy'de kullanicinin sectigi DNS ile cozulur ve "Akilli liste" ada
//! gore karar verir: listedeki sitelerin ilk mesaji bolunur, digerleri oldugu gibi gecer.

use std::net::SocketAddr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::thread::JoinHandle;

use macservice::sysproxy::SystemProxy;
use shared::t;
use tun2proxy::{ArgDns, Args, CancellationToken};

use crate::bridge;

/// Java'daki `Builder.setMtu` ile ayni.
const MTU: u16 = 1500;

#[derive(Default)]
pub struct AndroidVpn {
    tun: Option<Tun>,
}

struct Tun {
    fd: OwnedFd,
    proxy: SocketAddr,
    token: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

impl Tun {
    fn start(fd: OwnedFd, proxy: SocketAddr) -> Result<Tun, String> {
        let params = format!("socks5://{proxy}").parse().map_err(|e| format!("{e}"))?;
        let mut args = Args::default();
        args.proxy(params).tun_fd(Some(fd.as_raw_fd())).close_fd_on_drop(false).dns(ArgDns::Virtual).ipv6_enabled(true).verbosity(log::LevelFilter::Warn);
        let token = CancellationToken::new();
        let thread = {
            let token = token.clone();
            std::thread::Builder::new()
                .name("vpn".into())
                .spawn(move || run(args, token))
                .map_err(|e| t!("VPN iş parçacığı açılamadı: {e}", "Couldn't start the VPN thread: {e}"))?
        };
        macservice::info!("VPN arayüzü proxy'ye bağlandı ({proxy}).");
        Ok(Tun { fd, proxy, token, thread: Some(thread) })
    }

    fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// tun2proxy'yi durdurur; arayuzun dosya tanitici geri verilir.
    fn stop(mut self) -> OwnedFd {
        self.token.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.fd
    }
}

/// tun2proxy'nin kendi zamanlayicisi (tokio) bu is parcaciginda kurulur ve durdurulana kadar surer.
fn run(args: Args, token: CancellationToken) {
    let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("vpn-io").enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            macservice::error!("VPN zamanlayıcısı kurulamadı: {e}");
            return;
        }
    };
    let result = rt.block_on(tun2proxy::general_run_async(args, MTU, false, token.clone()));
    if token.is_cancelled() {
        return;
    }
    // Kendiliginden durdu: arayuz acik kalirsa trafik hicbir yere gitmez; baglanti kesilsin.
    match result {
        Ok(_) => macservice::error!("VPN aktarımı beklenmedik şekilde durdu."),
        Err(e) => macservice::error!("VPN aktarımı durdu: {e}"),
    }
    // Kesme bu is parcacigini da bekler (join); kendi kendini beklememesi icin ayri is parcaciginda.
    if let Some(ctrl) = crate::engine_host::running() {
        std::thread::spawn(move || ctrl.disconnect());
    }
}

impl SystemProxy for AndroidVpn {
    /// Arayuz yoksa VPN hizmetini baslatir; varsa (ayar degisti, proxy yeniden basladi) yalnizca
    /// proxy adresi degistiyse aktarmayi yeni adrese yeniden kurar.
    fn enable(&mut self, _pac_url: &str, proxy: SocketAddr) -> Result<(), String> {
        if let Some(tun) = self.tun.take() {
            if tun.proxy == proxy && tun.running() {
                self.tun = Some(tun);
                return Ok(());
            }
            let fd = tun.stop();
            return match Tun::start(fd, proxy) {
                Ok(t) => {
                    self.tun = Some(t);
                    Ok(())
                }
                Err(e) => {
                    bridge::stop_vpn();
                    Err(e)
                }
            };
        }
        let raw: RawFd = bridge::start_vpn(&t!("Korumalı", "Protected"), "DPI UI", &t!("Bağlantıyı kes", "Disconnect"))?;
        // SAFETY: Java tarafi tanitici `detachFd` ile birakti; sahibi artik biziz.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        match Tun::start(fd, proxy) {
            Ok(t) => {
                self.tun = Some(t);
                Ok(())
            }
            Err(e) => {
                bridge::stop_vpn();
                Err(e)
            }
        }
    }

    fn disable(&mut self) -> Result<(), String> {
        if let Some(tun) = self.tun.take() {
            drop(tun.stop());
            macservice::info!("VPN arayüzü kapatıldı.");
        }
        // Hizmet zaten kapanmis olabilir (sistem geri aldi); durdurmak yine de zararsiz.
        bridge::stop_vpn();
        Ok(())
    }
}
