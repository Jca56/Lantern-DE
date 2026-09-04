//! The Claude Code IDE bridge: a WebSocket server speaking MCP that the
//! `claude` CLI connects to, found through a lock file in
//! `~/.claude/ide/` and the environment our terminals hand it. Threads
//! here answer the protocol chatter (`initialize`, `tools/list`); tool
//! calls go to the app on the main thread, which answers when it can
//! (a diff, only once the user has decided).

pub mod diff;
mod sha1;
pub mod tools;
pub mod ws;

use std::io;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use lntrn_app::Waker;
use lntrn_core::{log_info, log_warn};

use crate::json::Json;
use crate::obj;

pub type ClientId = u64;
const IDE_NAME: &str = "lntrn-code";

/// What the connection threads hand the app.
pub enum Incoming {
    Connected,
    /// A `tools/call` for the app to answer with [`Bridge::respond`].
    Call { client: ClientId, id: Json, name: String, args: Json },
    Disconnected,
}

struct Client {
    id: ClientId,
    stream: Arc<Mutex<TcpStream>>,
}

pub struct Bridge {
    pub port: u16,
    token: String,
    lock_path: PathBuf,
    rx: Receiver<Incoming>,
    clients: Arc<Mutex<Vec<Client>>>,
    /// The project folder and the folders our terminals sit in: where a
    /// `claude` may be started from and still find us.
    roots: Vec<PathBuf>,
}

/// `$CLAUDE_CONFIG_DIR/ide` or `~/.claude/ide`.
fn ide_dir() -> Option<PathBuf> {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))?;
    Some(base.join("ide"))
}

/// A token in UUID form, from the clock and the pid.
fn random_token() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64;
    let mut rng = lntrn_core::Pcg32::new(nanos ^ (u64::from(std::process::id()) << 32));
    let words: Vec<u32> = (0..4).map(|_| rng.next_u32()).collect();
    let hex = format!("{:08x}{:08x}{:08x}{:08x}", words[0], words[1], words[2], words[3]);
    format!("{}-{}-4{}-{}-{}", &hex[0..8], &hex[8..12], &hex[13..16], &hex[16..20], &hex[20..32])
}

/// Lock files of ours whose process is gone.
fn sweep_stale(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for e in read.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "lock") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = Json::parse(&text) else {
            continue;
        };
        let pid = json.get("pid").and_then(Json::int).unwrap_or(0);
        if json.field_str("ideName") == IDE_NAME && (pid <= 0 || !Path::new(&format!("/proc/{pid}")).exists()) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

impl Bridge {
    /// Bind a port, write the lock file and start accepting.
    pub fn start(root: Option<&Path>, waker: Option<Waker>) -> io::Result<Self> {
        let dir = ide_dir().ok_or_else(|| io::Error::other("no home directory"))?;
        std::fs::create_dir_all(&dir)?;
        sweep_stale(&dir);
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let (tx, rx) = channel();
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        let bridge = Self { port, token: token.clone(), lock_path: dir.join(format!("{port}.lock")), rx, clients: Arc::clone(&clients), roots: root.map(Path::to_path_buf).into_iter().collect() };
        bridge.write_lock()?;
        thread::Builder::new().name("ide-accept".into()).spawn(move || {
            for (id, stream) in (1..).zip(listener.incoming()) {
                let Ok(stream) = stream else {
                    break;
                };
                let (token, tx, clients, waker) = (token.clone(), tx.clone(), Arc::clone(&clients), waker.clone());
                let _ = thread::Builder::new().name(format!("ide-client-{id}")).spawn(move || serve(stream, id, &token, tx, clients, waker));
            }
        })?;
        log_info!("ide bridge: listening on 127.0.0.1:{port}");
        Ok(bridge)
    }

    fn write_lock(&self) -> io::Result<()> {
        let folders: Vec<Json> = self.roots.iter().map(|r| Json::from(r.display().to_string())).collect();
        let json = obj! {
            "pid" => std::process::id(),
            "workspaceFolders" => folders,
            "ideName" => IDE_NAME,
            "transport" => "ws",
            "runningInWindows" => false,
            "authToken" => self.token.as_str(),
        };
        std::fs::write(&self.lock_path, json.to_text())
    }

    /// The folders changed: the lock file says so for the next connection.
    pub fn set_roots(&mut self, roots: Vec<PathBuf>) {
        if roots != self.roots {
            self.roots = roots;
            if let Err(e) = self.write_lock() {
                log_warn!("ide bridge: lock file: {e}");
            }
        }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// What a terminal's environment needs so `claude` finds us.
    pub fn env(&self) -> Vec<(String, String)> {
        vec![("CLAUDE_CODE_SSE_PORT".into(), self.port.to_string()), ("ENABLE_IDE_INTEGRATION".into(), "true".into())]
    }

    pub fn connected(&self) -> usize {
        self.clients.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Everything that arrived since the last call.
    pub fn poll(&self) -> Vec<Incoming> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            out.push(m);
        }
        out
    }

    fn send_to(&self, client: ClientId, msg: &Json) {
        let stream = self.clients.lock().ok().and_then(|c| c.iter().find(|c| c.id == client).map(|c| Arc::clone(&c.stream)));
        if let Some(s) = stream {
            send(&s, msg);
        }
    }

    pub fn respond(&self, client: ClientId, id: &Json, result: Json) {
        self.send_to(client, &obj! { "jsonrpc" => "2.0", "id" => id.clone(), "result" => result });
    }

    /// A notification to every connected CLI.
    pub fn notify(&self, method: &str, params: Json) {
        let msg = obj! { "jsonrpc" => "2.0", "method" => method, "params" => params };
        let streams: Vec<Arc<Mutex<TcpStream>>> = self.clients.lock().map(|c| c.iter().map(|c| Arc::clone(&c.stream)).collect()).unwrap_or_default();
        for s in streams {
            send(&s, &msg);
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// The first part of a protocol message, for the log.
fn brief(text: &str) -> String {
    let cut = text.char_indices().nth(400).map_or(text.len(), |(i, _)| i);
    if cut < text.len() { format!("{}…", &text[..cut]) } else { text.to_owned() }
}

fn send(stream: &Arc<Mutex<TcpStream>>, msg: &Json) {
    let text = msg.to_text();
    // Every caret move is a selection notice; those stay out of the log.
    if msg.field_str("method") != "selection_changed" {
        log_info!("ide ← {}", brief(&text));
    }
    if let Ok(mut s) = stream.lock() {
        let _ = ws::write_text(&mut s, &text);
    }
}

fn reply(stream: &Arc<Mutex<TcpStream>>, id: Json, result: Json) {
    send(stream, &obj! { "jsonrpc" => "2.0", "id" => id, "result" => result });
}

fn serve(mut stream: TcpStream, id: ClientId, token: &str, tx: Sender<Incoming>, clients: Arc<Mutex<Vec<Client>>>, waker: Option<Waker>) {
    if let Err(e) = ws::handshake(&mut stream, token) {
        log_warn!("ide bridge: refused a connection: {e}");
        return;
    }
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let shared = Arc::new(Mutex::new(writer));
    if let Ok(mut c) = clients.lock() {
        c.push(Client { id, stream: Arc::clone(&shared) });
    }
    let wake = || {
        if let Some(w) = &waker {
            w.wake();
        }
    };
    let _ = tx.send(Incoming::Connected);
    wake();
    log_info!("ide bridge: client {id} connected");
    loop {
        match ws::read_message(&mut stream) {
            Ok(ws::Message::Text(text)) => {
                if let Some(call) = handle(&text, id, &shared) {
                    let _ = tx.send(call);
                    wake();
                }
            }
            Ok(ws::Message::Ping(p)) => {
                if let Ok(mut s) = shared.lock() {
                    let _ = ws::write_pong(&mut s, &p);
                }
            }
            Ok(ws::Message::Pong | ws::Message::Binary) => {}
            Ok(ws::Message::Close) => {
                log_info!("ide bridge: client {id} sent close");
                break;
            }
            Err(e) => {
                log_info!("ide bridge: client {id} read failed: {e}");
                break;
            }
        }
    }
    if let Ok(mut s) = shared.lock() {
        let _ = ws::write_close(&mut s);
    }
    if let Ok(mut c) = clients.lock() {
        c.retain(|c| c.id != id);
    }
    let _ = tx.send(Incoming::Disconnected);
    wake();
    log_info!("ide bridge: client {id} gone");
}

/// One JSON-RPC message: answered here when no app state is needed,
/// else handed back as a call for the app.
fn handle(text: &str, client: ClientId, stream: &Arc<Mutex<TcpStream>>) -> Option<Incoming> {
    log_info!("ide → {}", brief(text));
    let msg = Json::parse(text).ok()?;
    let method = msg.field_str("method").to_owned();
    let rid = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Json::Null);
    match (method.as_str(), rid) {
        ("initialize", Some(rid)) => reply(stream, rid, tools::initialize_result(&params)),
        ("ping", Some(rid)) => reply(stream, rid, Json::Obj(Vec::new())),
        ("tools/list", Some(rid)) => reply(stream, rid, tools::list()),
        ("tools/call", Some(rid)) => {
            let name = params.field_str("name").to_owned();
            let args = params.get("arguments").cloned().unwrap_or(Json::Obj(Vec::new()));
            return Some(Incoming::Call { client, id: rid, name, args });
        }
        (_, Some(rid)) => send(stream, &obj! { "jsonrpc" => "2.0", "id" => rid, "error" => obj! { "code" => -32601i64, "message" => format!("method not found: {method}") } }),
        _ => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn masked(payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x81];
        let len = payload.len();
        if len < 126 {
            out.push(0x80 | len as u8);
        } else {
            out.push(0x80 | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        }
        let mask = [7u8, 8, 9, 10];
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        out
    }

    fn read_text(c: &mut TcpStream) -> String {
        let mut head = [0u8; 2];
        c.read_exact(&mut head).unwrap();
        let mut len = usize::from(head[1] & 0x7F);
        if len == 126 {
            let mut b = [0u8; 2];
            c.read_exact(&mut b).unwrap();
            len = usize::from(u16::from_be_bytes(b));
        }
        let mut body = vec![0u8; len];
        c.read_exact(&mut body).unwrap();
        String::from_utf8(body).unwrap()
    }

    /// The whole handshake a CLI does, against a bridge with a scratch
    /// config dir: lock file, upgrade with the token, initialize, the
    /// tool list, and a tool call answered by the app side.
    #[test]
    fn cli_round_trip() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-ide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: the test owns this process's environment.
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", &dir) };
        let bridge = Bridge::start(Some(Path::new("/tmp/proj")), None).unwrap();
        let lock = std::fs::read_to_string(dir.join("ide").join(format!("{}.lock", bridge.port))).unwrap();
        let lock = Json::parse(&lock).unwrap();
        assert_eq!(lock.field_str("ideName"), "lntrn-code");
        assert_eq!(lock.get("pid").and_then(Json::int), Some(i64::from(std::process::id())));
        let token = lock.field_str("authToken").to_owned();
        assert_eq!(bridge.env()[0].1, bridge.port.to_string());

        let mut c = TcpStream::connect(("127.0.0.1", bridge.port)).unwrap();
        c.write_all(format!("GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{}: {token}\r\n\r\n", ws::AUTH_HEADER).as_bytes()).unwrap();
        let mut resp = [0u8; 300];
        let n = c.read(&mut resp).unwrap();
        assert!(String::from_utf8_lossy(&resp[..n]).starts_with("HTTP/1.1 101"));
        c.write_all(&masked(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#)).unwrap();
        let init = Json::parse(&read_text(&mut c)).unwrap();
        assert_eq!(init.get("result").unwrap().field_str("protocolVersion"), "2025-06-18");
        c.write_all(&masked(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)).unwrap();
        c.write_all(&masked(br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)).unwrap();
        let list = Json::parse(&read_text(&mut c)).unwrap();
        assert_eq!(list.get("result").unwrap().get("tools").unwrap().arr().unwrap().len(), 11);
        c.write_all(&masked(br#"{"jsonrpc":"2.0","id":"c1","method":"tools/call","params":{"name":"getWorkspaceFolders","arguments":{}}}"#)).unwrap();
        // The call reaches the app side; it answers.
        let mut got = None;
        for _ in 0..50 {
            for m in bridge.poll() {
                if let Incoming::Call { client, id, name, args } = m {
                    got = Some((client, id, name, args));
                }
            }
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (client, id, name, _args) = got.expect("the call arrived");
        assert_eq!(name, "getWorkspaceFolders");
        assert_eq!(id, Json::Str("c1".into()));
        assert_eq!(bridge.connected(), 1);
        bridge.respond(client, &id, tools::text("ok"));
        let reply = Json::parse(&read_text(&mut c)).unwrap();
        assert_eq!(reply.get("id"), Some(&Json::Str("c1".into())));
        assert_eq!(reply.get("result").unwrap().get("content").unwrap().arr().unwrap()[0].field_str("text"), "ok");
        bridge.notify("selection_changed", obj! { "text" => "x" });
        let note = Json::parse(&read_text(&mut c)).unwrap();
        assert_eq!(note.field_str("method"), "selection_changed");
        // An unknown method gets an error; a wrong token is refused.
        c.write_all(&masked(br#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#)).unwrap();
        let err = Json::parse(&read_text(&mut c)).unwrap();
        assert_eq!(err.get("error").unwrap().get("code").and_then(Json::int), Some(-32601));
        let mut bad = TcpStream::connect(("127.0.0.1", bridge.port)).unwrap();
        bad.write_all(format!("GET / HTTP/1.1\r\nUpgrade: websocket\r\nSec-WebSocket-Key: a\r\n{}: wrong\r\n\r\n", ws::AUTH_HEADER).as_bytes()).unwrap();
        let n = bad.read(&mut resp).unwrap();
        assert!(String::from_utf8_lossy(&resp[..n]).starts_with("HTTP/1.1 401"));
        let lock_path = bridge.lock_path.clone();
        drop(bridge);
        assert!(!lock_path.exists(), "the lock file goes with the bridge");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
