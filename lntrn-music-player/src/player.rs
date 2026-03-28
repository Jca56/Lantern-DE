use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Player {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Sink,
    volume: f32,
    has_source: bool,
    play_start: Option<Instant>,
    elapsed_at_pause: Duration,
}

#[allow(dead_code)]
impl Player {
    pub fn new(volume: f32) -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        let sink = Sink::try_new(&handle).ok()?;
        sink.set_volume(volume);
        Some(Self {
            _stream: stream,
            stream_handle: handle,
            sink,
            volume,
            has_source: false,
            play_start: None,
            elapsed_at_pause: Duration::ZERO,
        })
    }

    pub fn play_file(&mut self, path: &Path) -> Result<(), String> {
        self.sink.stop();

        let sink = Sink::try_new(&self.stream_handle).map_err(|e| e.to_string())?;
        sink.set_volume(self.volume);

        let file = File::open(path).map_err(|e| e.to_string())?;
        let source = Decoder::new(BufReader::new(file)).map_err(|e| e.to_string())?;
        sink.append(source);

        self.sink = sink;
        self.has_source = true;
        self.play_start = Some(Instant::now());
        self.elapsed_at_pause = Duration::ZERO;
        Ok(())
    }

    pub fn toggle_pause(&mut self) {
        if self.sink.is_paused() {
            self.sink.play();
            self.play_start = Some(Instant::now());
        } else {
            if let Some(start) = self.play_start.take() {
                self.elapsed_at_pause += start.elapsed();
            }
            self.sink.pause();
        }
    }

    pub fn stop(&mut self) {
        self.sink.stop();
        self.has_source = false;
        self.play_start = None;
        self.elapsed_at_pause = Duration::ZERO;
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 1.0);
        self.sink.set_volume(self.volume);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    pub fn is_playing(&self) -> bool {
        self.has_source && !self.sink.is_paused() && !self.sink.empty()
    }

    pub fn track_finished(&self) -> bool {
        self.has_source && self.sink.empty()
    }

    pub fn clear_finished(&mut self) {
        self.has_source = false;
        self.play_start = None;
        self.elapsed_at_pause = Duration::ZERO;
    }

    pub fn elapsed(&self) -> Duration {
        let running = match self.play_start {
            Some(start) => start.elapsed(),
            None => Duration::ZERO,
        };
        self.elapsed_at_pause + running
    }
}
