// A small HTTP server on localhost that plays Firestore and Storage: it
// records every request (as far as it arrived) and answers from a script.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::super::http::Authed;
use super::super::{CloudConfig, Session};

#[derive(Debug, Clone)]
pub(super) struct Request {
    pub method: String,
    /// Path and query, as sent.
    pub target: String,
    /// The header block, lowercased.
    head: String,
    pub body: Vec<u8>,
    /// Did the whole declared body arrive before the connection ended?
    pub complete: bool,
}

impl Request {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("a JSON body")
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.head
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{name}: ")))
    }
}

type Handler = Box<dyn FnMut(&Request) -> (u16, String) + Send>;

pub(super) struct Server {
    url: String,
    seen: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn read_request(stream: &mut TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return None,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
        }
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_lowercase();
    let first = String::from_utf8_lossy(&raw[..head_end]).lines().next()?.to_string();
    let mut parts = first.split(' ');
    let (method, target) = (parts.next()?.to_string(), parts.next()?.to_string());
    let declared: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length: "))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let mut body = raw[head_end..].to_vec();
    while body.len() < declared {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
        }
    }
    Some(Request {
        method,
        target,
        head,
        complete: body.len() == declared,
        body,
    })
}

impl Server {
    /// `handler` answers each complete request with (status, body). A
    /// request whose body stops short gets no answer, like a real server
    /// that never saw the end of it.
    fn start(mut handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen_thr, stop_thr) = (seen.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !stop_thr.load(Ordering::Relaxed) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                };
                stream.set_nonblocking(false).unwrap();
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                seen_thr.lock().unwrap().push(request.clone());
                if !request.complete {
                    continue;
                }
                let (code, body) = handler(&request);
                let _ = write!(
                    stream,
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            url,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    /// A server that gives the answers in `script`, in order.
    pub fn scripted(script: Vec<(u16, String)>) -> Self {
        let mut script = script.into_iter();
        Self::start(Box::new(move |_| {
            script.next().unwrap_or((500, "script ran out".to_string()))
        }))
    }

    pub fn authed(&self) -> Authed {
        Authed {
            cfg: Arc::new(CloudConfig {
                api_key: "key".to_string(),
                project_id: "proj".to_string(),
                storage_bucket: "bucket".to_string(),
                firestore_url: self.url.clone(),
                storage_url: self.url.clone(),
            }),
            session: Arc::new(Mutex::new(Session {
                uid: "uid1".to_string(),
                email: "someone@example.com".to_string(),
                id_token: "the-id-token".to_string(),
                refresh_token: "refresh".to_string(),
                // Fresh: no request ever goes out to refresh it.
                expires_at: u64::MAX / 2,
            })),
        }
    }

    pub fn seen(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
