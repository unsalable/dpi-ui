//! Servis <-> arayuz protokolu: `\\.\pipe\GoodbyeDPI3` uzerinden satir basina bir
//! JSON mesaji. Istekler `id` tasir; yanitlar ayni `id` ile doner. Abone olan
//! istemciye servis kendiliginden olay da gonderir.
//!
//! Guvenlik: servis LocalSystem olarak calisir. Protokol bilerek yalnizca ayar
//! ve durum tasir; dosya yolu, URL ya da calistirilacak program asla almaz.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::update::UpdateStatus;

pub const PIPE_NAME: &str = r"\\.\pipe\GoodbyeDPI3";
pub const PROTOCOL_VERSION: u32 = 1;
/// Tek mesajin en buyuk boyutu: bozuk/kotu niyetli istemci bellegi sisiremesin.
pub const MAX_MESSAGE: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Request {
    Hello { client: String, protocol: u32 },
    GetState,
    SetConfig { config: Box<AppConfig> },
    Connect,
    Disconnect,
    GetStats,
    /// Bu baglantiya olay gonderilmeye baslansin.
    Subscribe,
    ForgetLearned { host: String },
    ClearLearned,
    GetLogs,
    /// Hazir sifreli DNS saglayicilarinin bu hattaki hizini olc (1-3 sn surer).
    BenchDns,
    /// Guncellemeyi denetle. `forced` degilse yakin zamanda denetlendiyse aga cikilmaz.
    /// Sonuc `Event::Update` olarak gelir.
    CheckUpdate { forced: bool },
    /// Indirilip dogrulanmis guncellemeyi simdi kur ("Otomatik yukle" kapaliyken kullanici ister).
    /// Kurulumu servis yapar; Windows onay sormaz.
    InstallUpdate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineState {
    #[default]
    Off,
    Starting,
    On,
    /// Calisiyor ama bir uygulama icin duraklatildi.
    Paused,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineStatus {
    pub state: EngineState,
    /// Hata ya da duraklatma aciklamasi (kullaniciya gosterilir).
    pub message: Option<String>,
    /// Baglanti kurulma zamani (Unix ms).
    pub since_ms: Option<u64>,
    pub method_name: String,
    pub dns_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsSnapshot {
    pub bypassed: u64,
    pub untouched: u64,
    pub fakes_sent: u64,
    pub quic_dropped: u64,
    pub voice_fakes_sent: u64,
    pub dns_queries: u64,
    pub dns_cached: u64,
    pub dns_failures: u64,
    pub learned: u64,
    pub engine_errors: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnedHost {
    pub host: String,
    pub reason: String,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchEntry {
    pub id: String,
    pub name: String,
    pub ok: u32,
    pub total: u32,
    pub median_ms: Option<u32>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// Cakisan surecin adi (or. winws.exe).
    pub process: String,
    pub pid: u32,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    pub service_version: String,
    pub engine: EngineStatus,
    pub config: AppConfig,
    pub learned: Vec<LearnedHost>,
    pub conflicts: Vec<Conflict>,
    #[serde(default)]
    pub update: UpdateStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Response {
    Ok,
    Hello { service_version: String, protocol: u32 },
    State { state: Box<StateSnapshot> },
    Stats { stats: StatsSnapshot },
    Logs { lines: Vec<String> },
    DnsBench { results: Vec<BenchEntry> },
    Error { message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Engine { status: EngineStatus },
    Config { config: Box<AppConfig> },
    Learned { host: LearnedHost },
    Conflicts { conflicts: Vec<Conflict> },
    Update { status: UpdateStatus },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientMessage {
    pub id: u64,
    pub request: Request,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerMessage {
    Reply { id: u64, response: Response },
    Event { event: Event },
}

/// Mesaji tek satir JSON olarak yazar.
pub fn write_message<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(msg).map_err(io::Error::other)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()
}

/// Bir satir okur ve cozer. Baglanti kapandiysa `Ok(None)`.
pub fn read_message<T: for<'de> Deserialize<'de>>(r: &mut impl BufRead, buf: &mut Vec<u8>) -> io::Result<Option<T>> {
    buf.clear();
    let mut limited = io::Read::take(&mut *r, MAX_MESSAGE as u64 + 1);
    let n = limited.read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "mesaj cok buyuk"));
    }
    serde_json::from_slice(buf).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let msgs = vec![
            ClientMessage { id: 1, request: Request::GetState },
            ClientMessage { id: 2, request: Request::SetConfig { config: Box::default() } },
            ClientMessage { id: 3, request: Request::ForgetLearned { host: "x.example".into() } },
        ];
        let mut wire = Vec::new();
        for m in &msgs {
            write_message(&mut wire, m).unwrap();
        }
        let mut r = io::BufReader::new(&wire[..]);
        let mut buf = Vec::new();
        for m in &msgs {
            let got: ClientMessage = read_message(&mut r, &mut buf).unwrap().unwrap();
            assert_eq!(&got, m);
        }
        assert!(read_message::<ClientMessage>(&mut r, &mut buf).unwrap().is_none());
    }

    #[test]
    fn rejects_oversized_and_garbage() {
        let big = vec![b'a'; MAX_MESSAGE + 10];
        let mut buf = Vec::new();
        assert!(read_message::<ClientMessage>(&mut io::BufReader::new(&big[..]), &mut buf).is_err());
        let junk = b"{\"id\":1,\"request\":{\"type\":\"rmrf\"}}\n";
        assert!(read_message::<ClientMessage>(&mut io::BufReader::new(&junk[..]), &mut buf).is_err());
    }

    /// Eksik alanlar varsayilana duser: en kisa durum yaniti da cozulur (testlerdeki sahte servis
    /// bunu gonderir) ve guncelleme durumu "kuruluyor" olabilir.
    #[test]
    fn minimal_state_reply_parses() {
        let line = br#"{"kind":"reply","id":3,"response":{"type":"state","state":{"serviceVersion":"9.9.9","engine":{},"config":{},"learned":[],"conflicts":[],"update":{"state":"installing","version":"9.9.9"}}}}"#;
        let msg: ServerMessage = serde_json::from_slice(line).unwrap();
        let ServerMessage::Reply { id: 3, response: Response::State { state } } = msg else { panic!("{msg:?}") };
        assert_eq!(state.update.state, crate::update::UpdateState::Installing { version: "9.9.9".into() });
        let ev = br#"{"kind":"event","event":{"type":"update","status":{"state":"installing","version":"9.9.9"}}}"#;
        assert!(serde_json::from_slice::<ServerMessage>(ev).is_ok());
    }

    #[test]
    fn server_message_shape() {
        let m = ServerMessage::Event { event: Event::Engine { status: EngineStatus::default() } };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"kind\":\"event\"") && s.contains("\"type\":\"engine\""));
    }
}
