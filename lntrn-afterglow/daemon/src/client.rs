//! The window's end of the helper's socket: one connection, opened when
//! first needed and again after any failure, since the helper is a
//! service that can restart under a window that stays open.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use lntrn_data::Doc;

use crate::tune::Tune;
use crate::wire::{self, Hello, MAX_LINE, Request, Status};

/// How long an answer may take: applying a tune writes a few dozen
/// files and a handful of driver calls, well inside this.
const PATIENCE: Duration = Duration::from_secs(10);

struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

pub struct Client {
    path: PathBuf,
    conn: Option<Conn>,
}

impl Client {
    pub fn new(path: impl Into<PathBuf>) -> Client {
        Client { path: path.into(), conn: None }
    }

    fn connect(&mut self) -> Result<&mut Conn, String> {
        if self.conn.is_none() {
            let stream = UnixStream::connect(&self.path).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => "the helper isn't running".to_owned(),
                std::io::ErrorKind::PermissionDenied => format!("the helper only answers the {} group", crate::GROUP),
                _ => format!("the helper can't be reached: {e}"),
            })?;
            stream.set_read_timeout(Some(PATIENCE)).map_err(|e| e.to_string())?;
            stream.set_write_timeout(Some(PATIENCE)).map_err(|e| e.to_string())?;
            let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
            self.conn = Some(Conn { reader, writer: stream });
        }
        Ok(self.conn.as_mut().expect("just connected"))
    }

    /// Send `request` and read its reply. Any failure drops the
    /// connection, so the next request starts a new one.
    fn ask(&mut self, request: &Request) -> Result<Doc, String> {
        let line = request.to_line();
        let conn = self.connect()?;
        let exchange = (|| -> std::io::Result<String> {
            conn.writer.write_all(line.as_bytes())?;
            conn.writer.write_all(b"\n")?;
            let mut answer = String::new();
            let n = (&mut conn.reader).take(MAX_LINE as u64).read_line(&mut answer)?;
            if n == 0 || !answer.ends_with('\n') {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the helper hung up"));
            }
            Ok(answer)
        })();
        match exchange {
            Ok(answer) => wire::reply(answer.trim_end()),
            Err(e) => {
                self.conn = None;
                Err(if e.kind() == std::io::ErrorKind::UnexpectedEof { "the helper hung up".to_owned() } else { format!("the helper didn't answer: {e}") })
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    pub fn hello(&mut self) -> Result<Hello, String> {
        Hello::from_doc(&self.ask(&Request::Hello)?)
    }

    pub fn status(&mut self) -> Result<Status, String> {
        Status::from_doc(&self.ask(&Request::Status)?)
    }

    /// Set `tune`, as a trial or for good. What couldn't be set, each
    /// said plainly; empty when all of it took.
    pub fn apply(&mut self, tune: &Tune, trial: bool) -> Result<Vec<String>, String> {
        self.ask(&Request::Apply { tune: tune.clone(), trial }).map(|d| problems(&d))
    }

    pub fn keep(&mut self) -> Result<(), String> {
        self.ask(&Request::Keep).map(drop)
    }

    pub fn revert(&mut self) -> Result<(), String> {
        self.ask(&Request::Revert).map(drop)
    }

    pub fn reset(&mut self) -> Result<Vec<String>, String> {
        self.ask(&Request::Reset).map(|d| problems(&d))
    }
}

/// The `problems` an `apply` or `reset` reply lists.
fn problems(doc: &Doc) -> Vec<String> {
    doc.get("problems").and_then(Doc::as_list).map(|l| l.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}
