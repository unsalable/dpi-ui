//! Yerel named pipe sunucusu (`\\.\pipe\GoodbyeDPI3`).
//!
//! - Yalnizca bu bilgisayardaki etkilesimli kullanicilar baglanabilir (SDDL + uzak
//!   istemci reddi). Ilk ornek bayragi baska bir surecin ayni adi kapmasini onler.
//! - Tutamaclar overlapped acilir: okuyucu ve yazici is parcaciklari birbirini
//!   beklemez (senkron pipe'ta okuma beklerken yazma da bekler ve kilitlenir).
//! - Yazici sinirli kuyrukla beslenir; yavas istemci servisi bekletmez. En fazla 8 istemci.

use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use shared::ipc::{self, ClientMessage, Request, ServerMessage};
use shared::pipeio::PipeHandle;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientSessionId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::CreateEventW;

use crate::controller::Controller;
use crate::{error, warn};

const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";
const MAX_CLIENTS: usize = 8;
static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

pub struct PipeServer {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn start(ctrl: Arc<Controller>) -> PipeServer {
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("pipe".into())
            .stack_size(128 * 1024)
            .spawn(move || accept_loop(ctrl, stop))
            .expect("pipe is parcacigi")
    };
    PipeServer { stop, thread: Some(thread) }
}

impl PipeServer {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Bekleyen ConnectNamedPipe'i uyandirmak icin kendimize baglaniriz.
        drop(PipeHandle::connect(ipc::PIPE_NAME, 500));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn create_instance(first: bool) -> HANDLE {
    let sddl = wide(PIPE_SDDL);
    let name = wide(ipc::PIPE_NAME);
    unsafe {
        let mut sd = std::ptr::null_mut();
        let ok = ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut());
        let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
        let h = CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 },
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            if ok != 0 { &sa } else { std::ptr::null() },
        );
        if ok != 0 {
            LocalFree(sd as _);
        }
        h
    }
}

/// Bir istemcinin baglanmasini bekler (overlapped ConnectNamedPipe).
fn wait_for_client(h: HANDLE) -> bool {
    unsafe {
        let event = CreateEventW(std::ptr::null(), 1, 0, std::ptr::null());
        if event.is_null() {
            return false;
        }
        let mut ov: OVERLAPPED = std::mem::zeroed();
        ov.hEvent = event;
        let connected = if ConnectNamedPipe(h, &mut ov) != 0 {
            true
        } else {
            match GetLastError() {
                ERROR_PIPE_CONNECTED => true,
                ERROR_IO_PENDING => {
                    let mut n = 0;
                    GetOverlappedResult(h, &ov, &mut n, 1) != 0
                }
                _ => false,
            }
        };
        CloseHandle(event);
        connected
    }
}

fn accept_loop(ctrl: Arc<Controller>, stop: Arc<AtomicBool>) {
    let active = Arc::new(AtomicUsize::new(0));
    let mut first = true;

    while !stop.load(Ordering::SeqCst) {
        let handle = create_instance(first);
        if handle == INVALID_HANDLE_VALUE {
            let code = unsafe { GetLastError() };
            error!("Pipe oluşturulamadı (Windows hata kodu {code}). Başka bir örnek çalışıyor olabilir.");
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        first = false;
        let pipe = unsafe { PipeHandle::from_raw(handle) };

        if !wait_for_client(handle) || stop.load(Ordering::SeqCst) {
            continue; // dusurulunce kapanir
        }
        if active.load(Ordering::SeqCst) >= MAX_CLIENTS {
            warn!("Çok fazla istemci; yeni bağlantı reddedildi.");
            continue;
        }

        let ctrl = ctrl.clone();
        let active = active.clone();
        active.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new().name("istemci".into()).stack_size(256 * 1024).spawn({
            let active = active.clone();
            move || {
                serve(ctrl, pipe);
                active.fetch_sub(1, Ordering::SeqCst);
            }
        });
        if spawned.is_err() {
            active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn serve(ctrl: Arc<Controller>, pipe: PipeHandle) {
    let id = NEXT_CLIENT.fetch_add(1, Ordering::Relaxed);
    // Istemcinin Windows oturumu: guncellemeden sonra arayuz ayni oturumda yeniden acilir.
    let mut session = 0u32;
    let session = (unsafe { GetNamedPipeClientSessionId(pipe.raw(), &mut session) } != 0).then_some(session);
    let Ok((reader, mut writer)) = pipe.split() else { return };
    let (tx, rx) = sync_channel::<ServerMessage>(64);
    // Yazici, kuyruk kapaninca ya da yazma basarisiz olunca biter.
    let spawned = std::thread::Builder::new().name("istemci-yazici".into()).stack_size(128 * 1024).spawn(move || {
        for msg in rx {
            if ipc::write_message(&mut writer, &msg).is_err() {
                break;
            }
        }
    });
    if spawned.is_err() {
        return;
    }

    let mut reader = BufReader::with_capacity(8 * 1024, reader);
    let mut buf = Vec::with_capacity(1024);
    loop {
        match ipc::read_message::<ClientMessage>(&mut reader, &mut buf) {
            Ok(Some(msg)) => {
                if let Request::Hello { client, .. } = &msg.request {
                    ctrl.register_client(id, session, client);
                }
                let response = ctrl.handle(msg.request, Some((id, &tx)));
                if tx.send(ServerMessage::Reply { id: msg.id, response }).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                warn!("Geçersiz istemci mesajı; bağlantı kapatıldı ({e}).");
                break;
            }
        }
    }
    // Aboneligi hemen sil: kuyrugun son gondericisi de dusunce yazici biter.
    ctrl.unsubscribe(id);
}
