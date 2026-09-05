//! One language server process: started with pipes, its messages read
//! on a thread and handed over as [`Incoming`], requests and
//! notifications written to it, its stderr kept in a log file.

use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

use lntrn_app::Waker;

use super::framing;
use crate::json::Json;
use crate::obj;

pub enum Incoming {
    Response { id: u64, result: Option<Json>, error: Option<String> },
    Notification { method: String, params: Json },
    /// The server asks something of us; `id` goes back in the answer.
    Request { id: Json, method: String, params: Json },
    /// The stream ended.
    Exited,
}

pub struct Client {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    rx: Receiver<Incoming>,
    next_id: u64,
}

fn log_path(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(format!(".lantern/log/lsp-{name}.log")))
}

impl Client {
    pub fn spawn(program: &str, args: &[&str], cwd: &Path, waker: Option<Waker>, name: &str) -> std::io::Result<Self> {
        let mut child = Command::new(program).args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| std::io::Error::other("no stderr"))?;
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        let (tx, rx) = channel();
        let w = waker.clone();
        // `LNTRN_LSP_TRACE=1` echoes every message to stderr.
        let trace = std::env::var_os("LNTRN_LSP_TRACE").is_some();
        let _ = std::thread::Builder::new().name(format!("lsp-{name}")).spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                match framing::read_message(&mut r) {
                    Ok(Some(bytes)) => {
                        if trace {
                            eprintln!("lsp<- {}", String::from_utf8_lossy(&bytes[..bytes.len().min(300)]));
                        }
                        if let Some(m) = parse(&bytes)
                            && tx.send(m).is_err()
                        {
                            break;
                        }
                    }
                    _ => {
                        let _ = tx.send(Incoming::Exited);
                        break;
                    }
                }
                if let Some(w) = &w {
                    w.wake();
                }
            }
            if let Some(w) = &w {
                w.wake();
            }
        });
        let log = log_path(name);
        let _ = std::thread::Builder::new().name(format!("lsp-{name}-err")).spawn(move || {
            use std::io::BufRead;
            let mut file = log.and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(f) = file.as_mut() {
                    let _ = writeln!(f, "{line}");
                }
            }
        });
        Ok(Self { child, stdin: Some(BufWriter::new(stdin)), rx, next_id: 1 })
    }

    fn write(&mut self, body: &Json) {
        let text = body.to_text();
        if std::env::var_os("LNTRN_LSP_TRACE").is_some() {
            eprintln!("lsp-> {}", &text[..text.len().min(300)]);
        }
        if let Some(w) = self.stdin.as_mut()
            && framing::write_message(w, text.as_bytes()).is_err()
        {
            self.stdin = None;
        }
    }

    pub fn request(&mut self, method: &str, params: Json) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&obj! { "jsonrpc" => "2.0", "id" => id, "method" => method, "params" => params });
        id
    }

    pub fn notify(&mut self, method: &str, params: Json) {
        self.write(&obj! { "jsonrpc" => "2.0", "method" => method, "params" => params });
    }

    pub fn respond(&mut self, id: &Json, result: Json) {
        self.write(&obj! { "jsonrpc" => "2.0", "id" => id.clone(), "result" => result });
    }

    pub fn poll(&mut self) -> Vec<Incoming> {
        let mut out = Vec::new();
        while let Ok(m) = self.rx.try_recv() {
            out.push(m);
        }
        out
    }

    pub fn alive(&self) -> bool {
        self.stdin.is_some()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.notify("exit", Json::Null);
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse(bytes: &[u8]) -> Option<Incoming> {
    let text = std::str::from_utf8(bytes).ok()?;
    let j = Json::parse(text).ok()?;
    let method = j.get("method").and_then(Json::str).map(str::to_owned);
    let id = j.get("id").cloned();
    match (method, id) {
        (Some(method), Some(id)) if id != Json::Null => Some(Incoming::Request { id, method, params: j.get("params").cloned().unwrap_or(Json::Null) }),
        (Some(method), _) => Some(Incoming::Notification { method, params: j.get("params").cloned().unwrap_or(Json::Null) }),
        (None, Some(id)) => {
            let id = id.num()? as u64;
            let error = j.get("error").map(|e| e.get("message").and_then(Json::str).unwrap_or("error").to_owned());
            Some(Incoming::Response { id, result: j.get("result").cloned(), error })
        }
        _ => None,
    }
}
