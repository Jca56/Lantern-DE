//! The default output: its volume and mute, the outputs there are to
//! pick from, and changing all three, through WirePlumber's `wpctl` (as
//! the volume keys do). `pactl subscribe`, one process for the life of
//! the thread, says the moment an output changes, whoever changed it, so
//! `wpctl` runs only then: nothing is polled. Without `pactl`, it reads
//! every few seconds instead.
//!
//! Volumes are fractions: 1.0 is 100%, and it goes to [`MAX`] (120%), as
//! the keys allow.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use crate::board::{Board, Notify};
use crate::{level, sys};

/// The loudest it goes: `wpctl --limit 1.2`, as the volume keys set it.
pub const MAX: f64 = 1.2;
/// How often it is read while `pactl` can't say when to.
const POLL: Duration = Duration::from_secs(3);
/// How often it is read anyway, in case an event went missing.
const REREAD: Duration = Duration::from_secs(60);
/// A burst of events (a new output brings several) is read once.
const SETTLE: Duration = Duration::from_millis(40);
/// Between attempts to start `pactl subscribe` again.
const RESPAWN: Duration = Duration::from_secs(5);
/// Outputs the list leaves out: a monitor's own sound, which is rarely
/// where the sound should go (as the Command Center lists them).
const HIDDEN: [&str; 2] = ["HDMI", "DisplayPort"];

/// One output (a PipeWire sink).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sink {
    /// What `wpctl set-default` takes.
    pub id: u32,
    pub name: String,
    pub default: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    /// `wpctl` answers: there is sound to control.
    pub present: bool,
    /// The default output's, 0 to [`MAX`].
    pub volume: f64,
    pub muted: bool,
    pub sinks: Vec<Sink>,
}

enum Cmd {
    Set(f64),
    Step { steps: i32, osd: bool },
    ToggleMute { osd: bool },
    Default(u32),
    /// `pactl` heard something change; `outputs` when it may be the list.
    Changed { outputs: bool },
}

pub struct Audio {
    board: Arc<Board<AudioState>>,
    tx: Sender<Cmd>,
}

impl Audio {
    /// Read the default output now (on the thread, which starts here),
    /// and keep it current.
    pub fn start(notify: Notify) -> Audio {
        let board = Board::new(AudioState::default(), notify);
        let (tx, rx) = channel();
        let live = Arc::new(AtomicBool::new(false));
        let (theirs, events, heard) = (board.clone(), tx.clone(), live.clone());
        let spawned = std::thread::Builder::new().name("audio".into()).spawn(move || run(&theirs, &rx, &heard));
        if let Err(e) = spawned {
            eprintln!("[lntrn-status] audio: no thread: {e}; there is no sound to control from here");
        }
        let spawned = std::thread::Builder::new().name("audio-events".into()).spawn(move || subscribe(&events, &live));
        if let Err(e) = spawned {
            eprintln!("[lntrn-status] audio: no events thread: {e}");
        }
        Audio { board, tx }
    }

    pub fn state(&self) -> AudioState {
        self.board.get()
    }

    /// Set the default output's volume (0 to [`MAX`]).
    pub fn set(&self, volume: f64) {
        let volume = volume.clamp(0.0, MAX);
        self.board.update(|s| s.volume = volume);
        let _ = self.tx.send(Cmd::Set(volume));
    }

    /// Turn it up (or down) by `steps` 5% steps, as the keys do; `osd`
    /// shows where it landed on Lantern's on-screen display, as they do.
    pub fn step(&self, steps: i32, osd: bool) {
        let _ = self.tx.send(Cmd::Step { steps, osd });
    }

    pub fn toggle_mute(&self, osd: bool) {
        self.board.update(|s| s.muted = !s.muted);
        let _ = self.tx.send(Cmd::ToggleMute { osd });
    }

    /// Send the sound to the output with this id.
    pub fn set_default(&self, id: u32) {
        self.board.update(|s| s.sinks.iter_mut().for_each(|k| k.default = k.id == id));
        let _ = self.tx.send(Cmd::Default(id));
    }
}

fn run(board: &Board<AudioState>, rx: &Receiver<Cmd>, live: &AtomicBool) {
    board.put(read(true));
    loop {
        let wait = if live.load(Ordering::Acquire) { REREAD } else { POLL };
        let first = match rx.recv_timeout(wait) {
            Ok(cmd) => cmd,
            Err(RecvTimeoutError::Timeout) => Cmd::Changed { outputs: !live.load(Ordering::Acquire) },
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let mut cmds = vec![first];
        // What came with it, and what follows close behind.
        while let Ok(cmd) = rx.recv_timeout(SETTLE) {
            cmds.push(cmd);
        }
        let (mut target, mut steps, mut mute, mut osd, mut outputs) = (None, 0, false, false, false);
        let mut default = None;
        for cmd in cmds {
            match cmd {
                // A slider dragged sends one a pixel: only the last matters.
                Cmd::Set(v) => (target, steps) = (Some(v), 0),
                Cmd::Step { steps: n, osd: show } => (steps, osd) = (steps + n, osd || show),
                Cmd::ToggleMute { osd: show } => (mute, osd) = (!mute, osd || show),
                Cmd::Default(id) => default = Some(id),
                Cmd::Changed { outputs: list } => outputs |= list,
            }
        }
        if let Some(id) = default {
            wpctl(&["set-default", &id.to_string()]);
            outputs = true;
        }
        if steps != 0 {
            let from = target.unwrap_or_else(|| board.get().volume);
            target = Some(level::step5(from, steps));
        }
        if let Some(v) = target {
            wpctl(&["set-volume", "--limit", "1.2", "@DEFAULT_AUDIO_SINK@", &format!("{:.2}", v.clamp(0.0, MAX))]);
        }
        if mute {
            wpctl(&["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]);
        }
        let mut now = read(outputs);
        if !outputs {
            now.sinks = board.get().sinks;
        }
        if osd && now.present {
            show(&now);
        }
        board.put(now);
    }
}

/// The default output as `wpctl` has it now, and the list of outputs
/// when `outputs`.
fn read(outputs: bool) -> AudioState {
    let Some((volume, muted)) = output(&["get-volume", "@DEFAULT_AUDIO_SINK@"]).as_deref().and_then(parse_volume) else {
        return AudioState::default();
    };
    let sinks = if outputs { output(&["status"]).map(|s| parse_sinks(&s)).unwrap_or_default() } else { Vec::new() };
    AudioState { present: true, volume, muted, sinks }
}

fn wpctl(args: &[&str]) {
    if let Err(e) = Command::new("wpctl").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status() {
        eprintln!("[lntrn-status] audio: wpctl {}: {e}", args.join(" "));
    }
}

/// What `wpctl` prints, when it succeeds.
fn output(args: &[&str]) -> Option<String> {
    let out = Command::new("wpctl").args(args).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Lantern's on-screen display, as the volume keys show it.
fn show(state: &AudioState) {
    let pct = ((state.volume * 100.0).round() as u32).to_string();
    if state.muted {
        sys::osd(&["mute"]);
    } else {
        sys::osd(&["volume", &pct]);
    }
}

/// Follow `pactl subscribe`, telling the worker what changed; start it
/// again when it ends. `live` says whether it is running.
fn subscribe(tx: &Sender<Cmd>, live: &AtomicBool) {
    let mut said = false;
    loop {
        let mut command = Command::new("pactl");
        command.arg("subscribe").env("LC_ALL", "C").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        let child = sys::die_with_parent(&mut command).spawn();
        match child {
            Ok(mut child) => {
                live.store(true, Ordering::Release);
                if let Some(out) = child.stdout.take() {
                    for line in BufReader::new(out).lines() {
                        let Ok(line) = line else { break };
                        if let Some(outputs) = classify(&line)
                            && tx.send(Cmd::Changed { outputs }).is_err()
                        {
                            let _ = child.kill();
                            let _ = child.wait();
                            return;
                        }
                    }
                }
                live.store(false, Ordering::Release);
                let _ = child.kill();
                let _ = child.wait();
            }
            Err(e) if !said => {
                said = true;
                eprintln!("[lntrn-status] audio: no pactl subscribe ({e}); reading every {}s instead", POLL.as_secs());
            }
            Err(_) => {}
        }
        std::thread::sleep(RESPAWN);
    }
}

/// What a `pactl subscribe` line means for us: `Some(false)` an output's
/// volume or mute, `Some(true)` the outputs or the default may have
/// changed, `None` nothing of ours (an app's own stream, say).
fn classify(line: &str) -> Option<bool> {
    let on = line.split_once(" on ")?.1;
    let kind = on.split(" #").next().unwrap_or(on).trim();
    match kind {
        "sink" => Some(line.contains("'new'") || line.contains("'remove'")),
        "server" | "card" => Some(true),
        _ => None,
    }
}

/// `wpctl get-volume`: `Volume: 0.65` or `Volume: 0.65 [MUTED]`.
fn parse_volume(s: &str) -> Option<(f64, bool)> {
    let rest = s.trim().strip_prefix("Volume:")?.trim();
    let v: f64 = rest.split_whitespace().next()?.parse().ok()?;
    Some((v, rest.contains("[MUTED]")))
}

/// The audio outputs of `wpctl status`: its first `Sinks:` (audio comes
/// before video), each `│  *   61. Name [vol: 0.65]`, the default
/// starred; up to the next section's heading.
fn parse_sinks(status: &str) -> Vec<Sink> {
    const HEADINGS: [&str; 5] = ["Devices:", "Sinks:", "Sources:", "Filters:", "Streams:"];
    let mut sinks = Vec::new();
    let mut inside = false;
    for line in status.lines() {
        if HEADINGS.iter().any(|h| line.contains(h)) {
            if inside {
                break;
            }
            inside = line.contains("Sinks:");
            continue;
        }
        if !inside {
            continue;
        }
        let entry: String = line.chars().skip_while(|c| !c.is_ascii_digit() && *c != '*').collect();
        let entry = entry.trim_start_matches('*').trim();
        let Some((id, rest)) = entry.split_once('.') else { continue };
        let Ok(id) = id.trim().parse() else { continue };
        let name = rest.rsplit_once('[').map_or(rest, |(name, _)| name).trim().to_owned();
        if name.is_empty() || HIDDEN.iter().any(|h| name.contains(h)) {
            continue;
        }
        sinks.push(Sink { id, name, default: line.contains('*') });
    }
    sinks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_volume_reads_with_and_without_mute() {
        assert_eq!(parse_volume("Volume: 0.65\n"), Some((0.65, false)));
        assert_eq!(parse_volume("Volume: 1.20 [MUTED]\n"), Some((1.2, true)));
        assert_eq!(parse_volume("nope"), None);
    }

    #[test]
    fn subscribe_lines_mean_what_they_say() {
        assert_eq!(classify("Event 'change' on sink #56"), Some(false));
        assert_eq!(classify("Event 'new' on sink #70"), Some(true));
        assert_eq!(classify("Event 'remove' on sink #70"), Some(true));
        assert_eq!(classify("Event 'change' on server #4294967295"), Some(true));
        assert_eq!(classify("Event 'change' on card #47"), Some(true));
        // An app's stream, a microphone, a client: not the outputs.
        assert_eq!(classify("Event 'change' on sink-input #91"), None);
        assert_eq!(classify("Event 'new' on source-output #12"), None);
        assert_eq!(classify("Event 'new' on client #300"), None);
    }

    #[test]
    fn the_audio_sinks_read_without_the_monitors_or_the_video() {
        let status = "PipeWire 'pipewire-0' [1.4.0, alva@genforge, cookie:1]\n \
└─ Clients:\n        31. WirePlumber\n\nAudio\n \
├─ Devices:\n │      47. Navi 31 HDMI/DP Audio               [alsa]\n │\n \
├─ Sinks:\n │      57. Navi 31 HDMI/DP Audio Digital Stereo (HDMI 3) [vol: 0.40]\n \
│  *   61. Starship/Matisse HD Audio Controller Analog Stereo [vol: 0.65]\n \
│      72. Buds Pro                            [vol: 1.00]\n │\n \
├─ Sources:\n │  *   62. Starship/Matisse HD Audio Controller Analog Stereo [vol: 1.00]\n\n\
Video\n ├─ Devices:\n ├─ Sinks:\n │      90. Not audio\n";
        let sinks = parse_sinks(status);
        assert_eq!(sinks, vec![Sink { id: 61, name: "Starship/Matisse HD Audio Controller Analog Stereo".into(), default: true }, Sink { id: 72, name: "Buds Pro".into(), default: false }]);
    }

    /// What this machine has: `cargo test -p lntrn-status -- --ignored --nocapture sound_here`.
    /// Reads only; nothing is played or changed.
    #[test]
    #[ignore = "runs this machine's wpctl"]
    fn sound_here_reads() {
        println!("{:#?}", read(true));
    }
}
