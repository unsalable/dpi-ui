//! WinDivert 2.2 icin ince sarmalayici.
//!
//! DLL derleme zamaninda baglanmaz: [`load`] ile verilen klasordeki `WinDivert.dll`
//! tam yoluyla yuklenir (arama yolu kacirmasina kapali). Surucu (`WinDivert64.sys`)
//! DLL ile ayni klasorde durmalidir; DLL onu kendi klasorunde arar.
//!
//! Yonetici gerektirmeyen yardimcilar ([`compile_filter`], [`eval_filter`],
//! [`calc_checksums`]) surucuyu hic yuklemez; testlerde kullanilir.

use std::ffi::{CStr, CString, c_char, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{FreeLibrary, GetLastError, HANDLE, HMODULE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};

pub const MAX_PACKET: usize = 0xFFFF;
/// `WinDivertRecvEx` ile tek cagrida alinabilecek en fazla paket (WINDIVERT_BATCH_MAX).
pub const BATCH_MAX: usize = 0xFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Layer {
    Network = 0,
    NetworkForward = 1,
    Flow = 2,
    Socket = 3,
    Reflect = 4,
}

pub mod flags {
    pub const SNIFF: u64 = 0x0001;
    pub const DROP: u64 = 0x0002;
    pub const RECV_ONLY: u64 = 0x0004;
    pub const SEND_ONLY: u64 = 0x0008;
    pub const NO_INSTALL: u64 = 0x0010;
    pub const FRAGMENTS: u64 = 0x0020;
}

#[derive(Clone, Copy, Debug)]
#[repr(u32)]
pub enum Param {
    QueueLength = 0,
    QueueTime = 1,
    QueueSize = 2,
    VersionMajor = 3,
    VersionMinor = 4,
}

#[derive(Clone, Copy, Debug)]
#[repr(u32)]
pub enum Shutdown {
    Recv = 1,
    Send = 2,
    Both = 3,
}

/// WINDIVERT_ADDRESS (80 bayt). Bit alanlari tek `bits` kelimesinde:
/// Layer:8, Event:8, Sniffed:1, Outbound:1, Loopback:1, Impostor:1, IPv6:1,
/// IPChecksum:1, TCPChecksum:1, UDPChecksum:1, Reserved1:8.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Address {
    pub timestamp: i64,
    bits: u32,
    reserved2: u32,
    data: [u32; 16],
}

const _: () = assert!(size_of::<Address>() == 80);

impl Default for Address {
    fn default() -> Self {
        Self { timestamp: 0, bits: 0, reserved2: 0, data: [0; 16] }
    }
}

impl std::fmt::Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Address")
            .field("outbound", &self.outbound())
            .field("ipv6", &self.ipv6())
            .field("impostor", &self.impostor())
            .field("if_idx", &self.if_idx())
            .finish()
    }
}

impl Address {
    const OUTBOUND: u32 = 1 << 17;
    const LOOPBACK: u32 = 1 << 18;
    const IMPOSTOR: u32 = 1 << 19;
    const IPV6: u32 = 1 << 20;

    fn flag(&self, bit: u32) -> bool {
        self.bits & bit != 0
    }

    fn set_flag(&mut self, bit: u32, on: bool) {
        if on { self.bits |= bit } else { self.bits &= !bit }
    }

    pub fn layer(&self) -> u8 {
        self.bits as u8
    }
    pub fn outbound(&self) -> bool {
        self.flag(Self::OUTBOUND)
    }
    pub fn set_outbound(&mut self, on: bool) {
        self.set_flag(Self::OUTBOUND, on)
    }
    pub fn loopback(&self) -> bool {
        self.flag(Self::LOOPBACK)
    }
    pub fn impostor(&self) -> bool {
        self.flag(Self::IMPOSTOR)
    }
    pub fn ipv6(&self) -> bool {
        self.flag(Self::IPV6)
    }
    pub fn set_ipv6(&mut self, on: bool) {
        self.set_flag(Self::IPV6, on)
    }
    /// Ag katmaninda arayuz dizini (IfIdx).
    pub fn if_idx(&self) -> u32 {
        self.data[0]
    }
    pub fn set_if_idx(&mut self, idx: u32) {
        self.data[0] = idx
    }
    pub fn sub_if_idx(&self) -> u32 {
        self.data[1]
    }
}

/// Windows hata kodu (GetLastError).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OsError(pub u32);

impl OsError {
    pub const ACCESS_DENIED: u32 = 5;
    pub const INVALID_HANDLE: u32 = 6;
    pub const INSUFFICIENT_BUFFER: u32 = 122;
    pub const NO_DATA: u32 = 232;
    pub const OPERATION_ABORTED: u32 = 995;
    pub const INVALID_IMAGE_HASH: u32 = 577;
    pub const DRIVER_BLOCKED: u32 = 1275;
    pub const DRIVER_FAILED_PRIOR_UNLOAD: u32 = 654;
    pub const SERVICE_DOES_NOT_EXIST: u32 = 1060;
    pub const FILE_NOT_FOUND: u32 = 2;

    fn last() -> Self {
        Self(unsafe { GetLastError() })
    }
}

impl std::fmt::Display for OsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Windows hata kodu {}", self.0)
    }
}

impl std::error::Error for OsError {}

#[derive(Debug)]
pub enum LoadError {
    /// DLL bulunamadi ya da yuklenemedi (Windows hata kodu ile).
    Library(OsError),
    /// DLL yuklendi ama beklenen islev yok (yanlis surum).
    MissingSymbol(&'static str),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Library(e) => write!(f, "WinDivert.dll yuklenemedi ({e})"),
            Self::MissingSymbol(s) => write!(f, "WinDivert.dll icinde {s} yok (uyumsuz surum)"),
        }
    }
}

impl std::error::Error for LoadError {}

type FnOpen = unsafe extern "C" fn(*const c_char, u32, i16, u64) -> HANDLE;
type FnRecvEx = unsafe extern "C" fn(HANDLE, *mut c_void, u32, *mut u32, u64, *mut Address, *mut u32, *mut c_void) -> i32;
type FnSendEx = unsafe extern "C" fn(HANDLE, *const c_void, u32, *mut u32, u64, *const Address, u32, *mut c_void) -> i32;
type FnShutdown = unsafe extern "C" fn(HANDLE, u32) -> i32;
type FnClose = unsafe extern "C" fn(HANDLE) -> i32;
type FnSetParam = unsafe extern "C" fn(HANDLE, u32, u64) -> i32;
type FnGetParam = unsafe extern "C" fn(HANDLE, u32, *mut u64) -> i32;
type FnCompile = unsafe extern "C" fn(*const c_char, u32, *mut c_char, u32, *mut *const c_char, *mut u32) -> i32;
type FnEval = unsafe extern "C" fn(*const c_char, *const c_void, u32, *const Address) -> i32;
type FnChecksums = unsafe extern "C" fn(*mut c_void, u32, *mut Address, u64) -> i32;

struct Api {
    module: HMODULE,
    open: FnOpen,
    recv_ex: FnRecvEx,
    send_ex: FnSendEx,
    shutdown: FnShutdown,
    close: FnClose,
    set_param: FnSetParam,
    get_param: FnGetParam,
    compile: FnCompile,
    eval: FnEval,
    checksums: FnChecksums,
}

// Islev isaretcileri ve modul tutamaci surec boyunca gecerli; DLL hic bosaltilmaz.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

static API: OnceLock<Api> = OnceLock::new();

/// `dir\WinDivert.dll` dosyasini yukler. Ilk basarili cagridan sonra ayni DLL kullanilir.
pub fn load(dir: &Path) -> Result<(), LoadError> {
    if API.get().is_some() {
        return Ok(());
    }

    let path: Vec<u16> = dir.join("WinDivert.dll").as_os_str().encode_wide().chain(Some(0)).collect();
    let module = unsafe {
        LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)
    };
    if module.is_null() {
        return Err(LoadError::Library(OsError::last()));
    }

    let api = unsafe { resolve(module) };
    match api {
        Ok(api) => {
            if API.set(api).is_err() {
                // Baska bir is parcacigi once davrandi; fazladan referansi birak.
                unsafe { FreeLibrary(module) };
            }
            Ok(())
        }
        Err(e) => {
            unsafe { FreeLibrary(module) };
            Err(e)
        }
    }
}

unsafe fn resolve(module: HMODULE) -> Result<Api, LoadError> {
    macro_rules! sym {
        ($name:literal) => {{
            let p = unsafe { GetProcAddress(module, concat!($name, "\0").as_ptr()) };
            match p {
                Some(f) => unsafe { std::mem::transmute(f) },
                None => return Err(LoadError::MissingSymbol($name)),
            }
        }};
    }

    Ok(Api {
        module,
        open: sym!("WinDivertOpen"),
        recv_ex: sym!("WinDivertRecvEx"),
        send_ex: sym!("WinDivertSendEx"),
        shutdown: sym!("WinDivertShutdown"),
        close: sym!("WinDivertClose"),
        set_param: sym!("WinDivertSetParam"),
        get_param: sym!("WinDivertGetParam"),
        compile: sym!("WinDivertHelperCompileFilter"),
        eval: sym!("WinDivertHelperEvalFilter"),
        checksums: sym!("WinDivertHelperCalcChecksums"),
    })
}

fn api() -> &'static Api {
    API.get().expect("windivert::load cagrilmadan WinDivert kullanildi")
}

/// DLL yuklu mu?
pub fn is_loaded() -> bool {
    API.get().is_some()
}

/// Yuklu DLL'in modul tutamaci (tani icin).
pub fn module_handle() -> Option<usize> {
    API.get().map(|a| a.module as usize)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterError {
    pub message: String,
    pub position: u32,
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (konum {})", self.message, self.position)
    }
}

impl std::error::Error for FilterError {}

fn c_filter(filter: &str) -> CString {
    // Filtre metni bizim urettigimiz ASCII; NUL icermez.
    CString::new(filter).unwrap_or_else(|_| CString::new("false").unwrap())
}

/// Filtreyi surucuye gitmeden derler; gecersizse hata metnini ve konumunu doner.
pub fn compile_filter(filter: &str, layer: Layer) -> Result<(), FilterError> {
    let f = c_filter(filter);
    let mut err: *const c_char = std::ptr::null();
    let mut pos: u32 = 0;
    let ok = unsafe { (api().compile)(f.as_ptr(), layer as u32, std::ptr::null_mut(), 0, &mut err, &mut pos) };
    if ok != 0 {
        return Ok(());
    }
    let message = if err.is_null() {
        "bilinmeyen hata".to_owned()
    } else {
        unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned()
    };
    Err(FilterError { message, position: pos })
}

/// Filtreyi verilen paket ve adres uzerinde kullanici modunda degerlendirir.
pub fn eval_filter(filter: &str, packet: &[u8], addr: &Address) -> bool {
    let f = c_filter(filter);
    unsafe { (api().eval)(f.as_ptr(), packet.as_ptr().cast(), packet.len() as u32, addr) != 0 }
}

/// IP/TCP/UDP/ICMP saglama toplamlarini yeniden hesaplar.
pub fn calc_checksums(packet: &mut [u8], addr: Option<&mut Address>, flags: u64) -> bool {
    let a = addr.map_or(std::ptr::null_mut(), |a| a as *mut Address);
    unsafe { (api().checksums)(packet.as_mut_ptr().cast(), packet.len() as u32, a, flags) != 0 }
}

/// Acik bir WinDivert tutamaci. Birden fazla is parcacigindan ayni anda
/// okunup yazilabilir (WinDivert bunu destekler). Dusurulunce kapanir; kapanan
/// tutamacin yakaladigi trafik otomatik olarak normal akisina doner.
pub struct Handle {
    raw: HANDLE,
}

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    pub fn open(filter: &str, layer: Layer, priority: i16, flags: u64) -> Result<Self, OsError> {
        let f = c_filter(filter);
        let raw = unsafe { (api().open)(f.as_ptr(), layer as u32, priority, flags) };
        if raw == INVALID_HANDLE_VALUE {
            return Err(OsError::last());
        }
        Ok(Self { raw })
    }

    /// Bir ya da daha fazla paketi (en fazla `addrs.len()`) `buf` icine art arda alir.
    /// Donen deger: (toplam bayt, paket sayisi). Paket sinirlari IP basliklarindan cozulur.
    pub fn recv_batch(&self, buf: &mut [u8], addrs: &mut [Address]) -> Result<(usize, usize), OsError> {
        let mut recv_len: u32 = 0;
        let mut addr_len: u32 = (addrs.len() * size_of::<Address>()) as u32;
        let ok = unsafe {
            (api().recv_ex)(
                self.raw,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut recv_len,
                0,
                addrs.as_mut_ptr(),
                &mut addr_len,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(OsError::last());
        }
        Ok((recv_len as usize, addr_len as usize / size_of::<Address>()))
    }

    /// Tek paket gonderir.
    pub fn send(&self, packet: &[u8], addr: &Address) -> Result<usize, OsError> {
        self.send_batch(packet, std::slice::from_ref(addr))
    }

    /// Art arda dizilmis paketleri, her biri icin bir adresle gonderir.
    pub fn send_batch(&self, packets: &[u8], addrs: &[Address]) -> Result<usize, OsError> {
        let mut sent: u32 = 0;
        let ok = unsafe {
            (api().send_ex)(
                self.raw,
                packets.as_ptr().cast(),
                packets.len() as u32,
                &mut sent,
                0,
                addrs.as_ptr(),
                (addrs.len() * size_of::<Address>()) as u32,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(OsError::last());
        }
        Ok(sent as usize)
    }

    pub fn set_param(&self, param: Param, value: u64) -> Result<(), OsError> {
        if unsafe { (api().set_param)(self.raw, param as u32, value) } == 0 {
            return Err(OsError::last());
        }
        Ok(())
    }

    pub fn param(&self, param: Param) -> Result<u64, OsError> {
        let mut v = 0u64;
        if unsafe { (api().get_param)(self.raw, param as u32, &mut v) } == 0 {
            return Err(OsError::last());
        }
        Ok(v)
    }

    /// Bekleyen `recv` cagrilarini `NO_DATA` ile dondurur (okuma dongusunu durdurmak icin).
    pub fn shutdown(&self, how: Shutdown) -> Result<(), OsError> {
        if unsafe { (api().shutdown)(self.raw, how as u32) } == 0 {
            return Err(OsError::last());
        }
        Ok(())
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { (api().close)(self.raw) };
    }
}

/// Art arda dizilmis paketlerin uzunluklarini IP basligindan cozer.
/// Bozuk/kesik veri gorulurse orada durur.
pub fn split_packets(buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = buf;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let len = match rest[0] >> 4 {
            4 if rest.len() >= 20 => u16::from_be_bytes([rest[2], rest[3]]) as usize,
            6 if rest.len() >= 40 => 40 + u16::from_be_bytes([rest[4], rest[5]]) as usize,
            _ => return None,
        };
        if len == 0 || len > rest.len() {
            return None;
        }
        let (head, tail) = rest.split_at(len);
        rest = tail;
        Some(head)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_bundled() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/windivert/2.2");
        load(&dir).expect("paketlenmis WinDivert.dll yuklenemedi");
    }

    /// 40 baytlik baslikla IPv4/TCP paketi: 10.0.0.2:50000 -> 1.2.3.4:443, verilen payload ile.
    fn tcp_packet(payload: &[u8]) -> Vec<u8> {
        let total = 40 + payload.len();
        let mut p = vec![0u8; total];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        p[8] = 64;
        p[9] = 6;
        p[12..16].copy_from_slice(&[10, 0, 0, 2]);
        p[16..20].copy_from_slice(&[1, 2, 3, 4]);
        p[20..22].copy_from_slice(&50000u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[32] = 0x50; // veri ofseti 5
        p[33] = 0x18; // PSH|ACK
        p[40..].copy_from_slice(payload);
        p
    }

    #[test]
    fn address_layout_bits() {
        let mut a = Address::default();
        a.set_outbound(true);
        assert!(a.outbound());
        assert!(!a.ipv6());
        a.set_ipv6(true);
        assert_eq!(a.bits, (1 << 17) | (1 << 20));
    }

    #[test]
    fn compiles_valid_filter_and_rejects_invalid() {
        load_bundled();
        assert!(compile_filter("outbound and tcp.DstPort == 443 and tcp.PayloadLength > 5", Layer::Network).is_ok());
        let err = compile_filter("outbound and tcp.Bogus == 1", Layer::Network).unwrap_err();
        assert!(err.position > 0, "{err}");
    }

    #[test]
    fn evaluates_filter_on_packet() {
        load_bundled();
        let hello = [0x16, 0x03, 0x01, 0x00, 0x10, 0x01, 0, 0, 0x0c];
        let mut p = tcp_packet(&hello);
        let mut addr = Address::default();
        addr.set_outbound(true);
        assert!(calc_checksums(&mut p, Some(&mut addr), 0));

        let f = "outbound and tcp and tcp.DstPort == 443 and tcp.Payload[0] == 0x16 and tcp.Payload[5] == 0x01";
        assert!(eval_filter(f, &p, &addr));
        assert!(!eval_filter("inbound", &p, &addr));
    }

    #[test]
    fn splits_concatenated_packets() {
        let a = tcp_packet(b"abc");
        let b = tcp_packet(b"hello world");
        let mut buf = a.clone();
        buf.extend_from_slice(&b);
        let parts: Vec<_> = split_packets(&buf).collect();
        assert_eq!(parts, vec![&a[..], &b[..]]);
        assert_eq!(split_packets(&buf[..10]).count(), 0);
    }
}
