//! Arayuzle konusma: veri klasorundeki Unix soketi (`ipc.sock`), Windows'taki named pipe'in
//! karsiligi. Mesajlar ayni: satir basina bir JSON (`shared::ipc`). Soket kullanicinin kendi
//! klasorunde ve yalnizca ona acik (0600); en fazla 8 istemci.

use std::io::BufReader;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread::JoinHandle;

use shared::ipc::{self, ClientMessage, ServerMessage};

use crate::controller::Controller;
use crate::warn;

const MAX_CLIENTS: usize = 8;
static NEXT_CLIENT: AtomicU64 = AtomicU64::new(1);

pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub fn start(ctrl: Arc<Controller>, path: &Path) -> std::io::Result<Server> {
    // Onceki calismadan kalan soket dosyasi baglanmayi engeller.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new().name("ipc".into()).stack_size(128 * 1024).spawn(move || accept_loop(listener, ctrl, stop))?
    };
    Ok(Server { path: path.to_owned(), stop, thread: Some(thread) })
}

impl Server {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn accept_loop(listener: UnixListener, ctrl: Arc<Controller>, stop: Arc<AtomicBool>) {
    let active = Arc::new(AtomicUsize::new(0));
    for conn in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = conn else { continue };
        if active.load(Ordering::SeqCst) >= MAX_CLIENTS {
            warn!("Çok fazla istemci; bağlantı reddedildi.");
            continue;
        }
        active.fetch_add(1, Ordering::SeqCst);
        let (ctrl, active) = (ctrl.clone(), active.clone());
        let spawned = std::thread::Builder::new().name("ipc-istemci".into()).stack_size(256 * 1024).spawn(move || {
            serve(stream, &ctrl);
            active.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            warn!("İstemci iş parçacığı açılamadı.");
        }
    }
}

fn serve(stream: UnixStream, ctrl: &Controller) {
    let id = NEXT_CLIENT.fetch_add(1, Ordering::Relaxed);
    let Ok(mut writer) = stream.try_clone() else { return };
    // Yazici kendi is parcaciginda: yavas istemci hizmeti bekletmez.
    let (tx, rx) = sync_channel::<ServerMessage>(64);
    let w = std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            if ipc::write_message(&mut writer, &msg).is_err() {
                return;
            }
        }
    });
    let mut reader = BufReader::with_capacity(16 * 1024, stream);
    let mut buf = Vec::with_capacity(4096);
    while let Ok(Some(msg)) = ipc::read_message::<ClientMessage>(&mut reader, &mut buf) {
        let response = ctrl.handle(msg.request, Some((id, &tx)));
        if tx.send(ServerMessage::Reply { id: msg.id, response }).is_err() {
            break;
        }
    }
    ctrl.unsubscribe(id);
    drop(tx);
    let _ = w.join();
}
