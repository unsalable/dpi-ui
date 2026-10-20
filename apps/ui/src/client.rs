//! Motor hizmetine IPC istemcisi. Arka plan is parcacigi baglantiyi kurar, kopunca
//! saniyede bir yeniden dener; istekler herhangi bir is parcacigindan gonderilebilir.
//! Geri cagrilar IPC is parcaciginda calisir (arayuze `invoke_from_event_loop` ile gecilir).

use std::collections::HashMap;
use std::io::BufReader;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shared::ipc::{self, ClientMessage, Event, Request, Response, ServerMessage, StateSnapshot};
use shared::pipeio::{PipeHandle, PipeWriter};

/// Servisin pipe adi. Testler (`GDPI3_TEST`) `GDPI3_PIPE` ile kendi sahte servislerine baglanir.
fn pipe_name() -> String {
    std::env::var_os("GDPI3_TEST")
        .and_then(|_| std::env::var("GDPI3_PIPE").ok())
        .filter(|p| p.starts_with(r"\\.\pipe\"))
        .unwrap_or_else(|| ipc::PIPE_NAME.to_owned())
}

pub enum Incoming {
    Connected(Box<StateSnapshot>),
    Disconnected,
    Event(Event),
}

type Callback = Box<dyn FnOnce(Response) + Send>;

struct Inner {
    writer: Mutex<Option<PipeWriter>>,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, Callback>>,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    pub fn start(name: &'static str, on: impl Fn(Incoming) + Send + Sync + 'static) -> Self {
        let inner = Arc::new(Inner { writer: Mutex::new(None), next: AtomicU64::new(1), pending: Mutex::new(HashMap::new()) });
        let client = Client { inner };
        let c = client.clone();
        std::thread::Builder::new()
            .name("ipc".into())
            .stack_size(256 * 1024)
            .spawn(move || c.run(name, on))
            .expect("ipc is parcacigi");
        client
    }

    pub fn request(&self, request: Request, cb: impl FnOnce(Response) + Send + 'static) {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        self.inner.pending.lock().unwrap().insert(id, Box::new(cb));
        let sent = {
            let mut w = self.inner.writer.lock().unwrap();
            match w.as_mut() {
                Some(w) => ipc::write_message(w, &ClientMessage { id, request }).is_ok(),
                None => false,
            }
        };
        if !sent {
            if let Some(cb) = self.inner.pending.lock().unwrap().remove(&id) {
                cb(Response::Error { message: "Motor hizmetine bağlanılamadı.".into() });
            }
        }
    }

    pub fn send(&self, request: Request) {
        self.request(request, |_| {});
    }

    fn run(&self, name: &str, on: impl Fn(Incoming)) {
        let mut announced_down = false;
        loop {
            let pipe = match PipeHandle::connect(&pipe_name(), 1000) {
                Ok(p) => p,
                Err(_) => {
                    if !announced_down {
                        on(Incoming::Disconnected);
                        announced_down = true;
                    }
                    std::thread::sleep(Duration::from_millis(1000));
                    continue;
                }
            };
            let Ok((reader, writer)) = pipe.split() else { continue };
            *self.inner.writer.lock().unwrap() = Some(writer);

            self.send(Request::Hello { client: name.into(), protocol: ipc::PROTOCOL_VERSION });
            self.send(Request::Subscribe);
            let (tx, rx) = std::sync::mpsc::channel();
            self.request(Request::GetState, move |r| {
                let _ = tx.send(r);
            });

            let mut reader = BufReader::with_capacity(16 * 1024, reader);
            let mut buf = Vec::with_capacity(4096);
            let mut first_state_sent = false;
            loop {
                match ipc::read_message::<ServerMessage>(&mut reader, &mut buf) {
                    Ok(Some(ServerMessage::Reply { id, response })) => {
                        if let Some(cb) = self.inner.pending.lock().unwrap().remove(&id) {
                            cb(response);
                        }
                        if !first_state_sent {
                            if let Ok(Response::State { state }) = rx.try_recv() {
                                on(Incoming::Connected(state));
                                first_state_sent = true;
                            }
                        }
                    }
                    Ok(Some(ServerMessage::Event { event })) => on(Incoming::Event(event)),
                    Ok(None) | Err(_) => break,
                }
            }

            *self.inner.writer.lock().unwrap() = None;
            let waiting: Vec<Callback> = self.inner.pending.lock().unwrap().drain().map(|(_, cb)| cb).collect();
            for cb in waiting {
                cb(Response::Error { message: "Motor hizmetiyle bağlantı koptu.".into() });
            }
            on(Incoming::Disconnected);
            announced_down = true;
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}
