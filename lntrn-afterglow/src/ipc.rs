//! GUI-side client for the lntrn-afterglowd line protocol.
//!
//! Holds one lazy connection to the root daemon's socket. Every request
//! writes a line and reads a line; on any I/O error the connection is dropped
//! so the next call transparently reconnects (the daemon is a boot service
//! that may restart, and the GUI can outlive it). The monitoring dashboard
//! only ever calls the read-only getters here.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

use crate::protocol::{self, get_num};
use crate::SOCKET_PATH;

/// Static hardware description — fetched once per connection.
#[derive(Debug, Clone, Default)]
pub struct Info {
    pub root: bool,
    pub cpu_model: String,
    pub gpu: bool,
    pub gpu_name: String,
    pub driver: String,
    pub power_min_w: u32,
    pub power_max_w: u32,
    pub power_default_w: u32,
    pub core_off: i32,
    pub core_min: i32,
    pub core_max: i32,
    pub mem_off: i32,
    pub mem_min: i32,
    pub mem_max: i32,
    pub fans: u32,
    pub rapl: bool,
    pub pl1_w: u32,
    pub pl2_w: u32,
    pub pl1_max_w: u32,
    pub pl2_max_w: u32,
}

/// One logical CPU's live state.
#[derive(Debug, Clone)]
pub struct CoreStat {
    pub id: usize,
    pub mhz: u32,
    pub util_pct: f32,
    pub is_pcore: bool,
}

/// Live telemetry — refreshed every poll tick. GPU fields stay zero on a
/// machine without an NVIDIA driver.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub has_gpu: bool,
    pub temp_c: u32,
    pub power_w: f32,
    pub power_limit_w: f32,
    pub core_mhz: u32,
    pub mem_mhz: u32,
    pub util_pct: u32,
    pub vram_used_mb: u64,
    pub vram_total_mb: u64,
    pub fans_pct: Vec<u32>,
    pub cpu_util_pct: f32,
    pub cpu_temp_c: Option<f32>,
    pub pkg_w: Option<f32>,
    pub cores: Vec<CoreStat>,
}

/// CPU governor / EPP / turbo state plus the available choices.
#[derive(Debug, Clone, Default)]
pub struct CpuTuning {
    pub governor: String,
    pub governors: Vec<String>,
    pub epp: String,
    pub epps: Vec<String>,
    pub turbo: Option<bool>,
    pub max_freq_pct: u32,
}

struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

#[derive(Default)]
pub struct Client {
    conn: Option<Conn>,
}

impl Client {
    pub fn new() -> Client {
        Client { conn: None }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    fn ensure(&mut self) -> Result<&mut Conn, String> {
        if self.conn.is_none() {
            let stream = UnixStream::connect(SOCKET_PATH)
                .map_err(|e| format!("afterglowd offline ({e})"))?;
            let reader = BufReader::new(
                stream
                    .try_clone()
                    .map_err(|e| format!("socket clone: {e}"))?,
            );
            self.conn = Some(Conn {
                reader,
                writer: stream,
            });
        }
        Ok(self.conn.as_mut().unwrap())
    }

    /// One request line in, one parsed response out. Drops the connection on
    /// any I/O failure so the next call reconnects.
    fn request(&mut self, line: &str) -> Result<HashMap<String, String>, String> {
        let conn = self.ensure()?;
        let io: std::io::Result<String> = (|| {
            writeln!(conn.writer, "{line}")?;
            conn.writer.flush()?;
            let mut resp = String::new();
            let n = conn.reader.read_line(&mut resp)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "daemon closed connection",
                ));
            }
            Ok(resp)
        })();
        match io {
            Ok(resp) => protocol::parse(resp.trim()),
            Err(e) => {
                self.conn = None;
                Err(format!("afterglowd I/O: {e}"))
            }
        }
    }

    /// Send a raw command line (`set …` / `reset`); Ok on `ok`, Err on `err`.
    pub fn command(&mut self, line: &str) -> Result<(), String> {
        self.request(line).map(|_| ())
    }

    pub fn info(&mut self) -> Result<Info, String> {
        let m = self.request("info")?;
        let mut i = Info {
            root: get_num::<u8>(&m, "root").unwrap_or(0) == 1,
            cpu_model: m.get("cpu_model").cloned().unwrap_or_default(),
            gpu: get_num::<u8>(&m, "gpu").unwrap_or(0) == 1,
            ..Info::default()
        };
        if i.gpu {
            i.gpu_name = m.get("name").cloned().unwrap_or_default();
            i.driver = m.get("driver").cloned().unwrap_or_default();
            i.power_min_w = get_num(&m, "pmin").unwrap_or(0);
            i.power_max_w = get_num(&m, "pmax").unwrap_or(0);
            i.power_default_w = get_num(&m, "pdef").unwrap_or(0);
            i.core_off = get_num(&m, "core_off").unwrap_or(0);
            i.core_min = get_num(&m, "core_min").unwrap_or(0);
            i.core_max = get_num(&m, "core_max").unwrap_or(0);
            i.mem_off = get_num(&m, "mem_off").unwrap_or(0);
            i.mem_min = get_num(&m, "mem_min").unwrap_or(0);
            i.mem_max = get_num(&m, "mem_max").unwrap_or(0);
            i.fans = get_num(&m, "fans").unwrap_or(0);
        }
        if get_num::<u8>(&m, "rapl").unwrap_or(0) == 1 {
            i.rapl = true;
            i.pl1_w = get_num(&m, "pl1").unwrap_or(0);
            i.pl2_w = get_num(&m, "pl2").unwrap_or(0);
            i.pl1_max_w = get_num(&m, "pl1_max").unwrap_or(0);
            i.pl2_max_w = get_num(&m, "pl2_max").unwrap_or(0);
        }
        Ok(i)
    }

    pub fn snapshot(&mut self) -> Result<Snapshot, String> {
        let m = self.request("snap")?;
        let mut s = Snapshot {
            has_gpu: m.contains_key("temp"),
            cpu_util_pct: get_num(&m, "cpu_util").unwrap_or(0.0),
            cpu_temp_c: get_num(&m, "cpu_temp"),
            pkg_w: get_num(&m, "pkg_w"),
            ..Snapshot::default()
        };
        if s.has_gpu {
            s.temp_c = get_num(&m, "temp").unwrap_or(0);
            s.power_w = get_num(&m, "power").unwrap_or(0.0);
            s.power_limit_w = get_num(&m, "plimit").unwrap_or(0.0);
            s.core_mhz = get_num(&m, "core").unwrap_or(0);
            s.mem_mhz = get_num(&m, "mem").unwrap_or(0);
            s.util_pct = get_num(&m, "util").unwrap_or(0);
            s.vram_used_mb = get_num(&m, "vram_used").unwrap_or(0);
            s.vram_total_mb = get_num(&m, "vram_total").unwrap_or(0);
            s.fans_pct = m
                .get("fan")
                .map(|v| v.split(',').filter_map(|p| p.parse().ok()).collect())
                .unwrap_or_default();
        }
        s.cores = parse_cores(m.get("cores").map(String::as_str).unwrap_or(""));
        Ok(s)
    }

    pub fn cpu_tuning(&mut self) -> Result<CpuTuning, String> {
        let m = self.request("cpu_tuning")?;
        Ok(CpuTuning {
            governor: m.get("governor").cloned().unwrap_or_default(),
            governors: split_csv(m.get("governors")),
            epp: m.get("epp").cloned().unwrap_or_default(),
            epps: split_csv(m.get("epps")),
            turbo: get_num::<u8>(&m, "turbo").map(|v| v == 1),
            max_freq_pct: get_num(&m, "max_freq_pct").unwrap_or(100),
        })
    }
}

/// Parses the `snap` cores field: "id:mhz:util:pcore" entries, comma-joined.
fn parse_cores(s: &str) -> Vec<CoreStat> {
    s.split(',')
        .filter(|e| !e.is_empty())
        .filter_map(|entry| {
            let mut f = entry.split(':');
            Some(CoreStat {
                id: f.next()?.parse().ok()?,
                mhz: f.next()?.parse().ok()?,
                util_pct: f.next()?.parse().ok()?,
                is_pcore: f.next()? == "1",
            })
        })
        .collect()
}

fn split_csv(v: Option<&String>) -> Vec<String> {
    v.map(|s| {
        s.split(',')
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}
