//! Looking at the machine: the graphics card and the processor read
//! straight from their drivers (anyone may), and the helper's word on
//! what is applied, into a [`Frame`] with the last two minutes of each
//! reading kept for graphs.

use std::sync::Arc;

use lntrn_afterglowd::hw::cpu::{self, Cpu};
use lntrn_afterglowd::hw::nvml::{self, Nvml};
use lntrn_afterglowd::tune::Limits;
use lntrn_afterglowd::wire::{Hello, Status};

/// How many readings a graph shows: two minutes, at one a second.
pub const HISTORY: usize = 120;

/// The graphics card's reading and its recent past.
#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub name: String,
    pub now: nvml::Reading,
    pub clock: Vec<f32>,
    pub celsius: Vec<f32>,
    pub watts: Vec<f32>,
    pub fan: Vec<f32>,
}

/// The processor's reading and its recent past.
#[derive(Clone, Debug, Default)]
pub struct Processor {
    pub now: cpu::Reading,
    pub usage: Vec<f32>,
    pub celsius: Vec<f32>,
    /// What the package draws, from the helper (it takes root to read).
    pub watts: Vec<f32>,
}

/// What the helper says, or why it says nothing.
#[derive(Clone, Debug)]
pub enum Helper {
    Away(String),
    Here { hello: Arc<Hello>, status: Status },
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub gpu: Option<Gpu>,
    pub cpu: Processor,
    pub helper: Helper,
}

impl Default for Frame {
    fn default() -> Self {
        Frame { gpu: None, cpu: Processor::default(), helper: Helper::Away("not asked yet".to_owned()) }
    }
}

impl Frame {
    pub fn hello(&self) -> Option<&Hello> {
        match &self.helper {
            Helper::Here { hello, .. } => Some(hello),
            Helper::Away(_) => None,
        }
    }

    pub fn status(&self) -> Option<&Status> {
        match &self.helper {
            Helper::Here { status, .. } => Some(status),
            Helper::Away(_) => None,
        }
    }

    pub fn limits(&self) -> Option<&Limits> {
        self.hello().map(|h| &h.limits)
    }

    /// The average clock of the performance cores and of the efficiency
    /// cores, MHz (`0` for a kind there is none of).
    pub fn core_clocks(&self) -> (f64, f64) {
        let mean = |pcore: bool| {
            let mhz: Vec<f64> = self.cpu.now.threads.iter().filter(|t| t.pcore == pcore && t.mhz > 0).map(|t| f64::from(t.mhz)).collect();
            if mhz.is_empty() { 0.0 } else { mhz.iter().sum::<f64>() / mhz.len() as f64 }
        };
        (mean(true), mean(false))
    }
}

fn push(history: &mut Vec<f32>, value: f32) {
    history.push(value);
    if history.len() > HISTORY {
        history.remove(0);
    }
}

pub struct Sampler {
    nvml: Option<Nvml>,
    cpu: Cpu,
    gpu: Option<Gpu>,
    processor: Processor,
}

impl Sampler {
    pub fn new() -> Sampler {
        let nvml = Nvml::open().ok();
        let gpu = nvml.as_ref().map(|n| Gpu { name: n.name().to_owned(), ..Gpu::default() });
        Sampler { nvml, cpu: Cpu::discover(), gpu, processor: Processor::default() }
    }

    /// Read everything once more, with what the helper said.
    pub fn sample(&mut self, helper: Helper) -> Frame {
        if let (Some(n), Some(g)) = (&self.nvml, &mut self.gpu) {
            g.now = n.read();
            push(&mut g.clock, g.now.core_mhz.unwrap_or(0) as f32);
            push(&mut g.celsius, g.now.celsius.unwrap_or(0) as f32);
            push(&mut g.watts, g.now.watts.unwrap_or(0.0));
            push(&mut g.fan, g.now.fans.first().copied().unwrap_or(0) as f32);
        }
        let p = &mut self.processor;
        p.now = self.cpu.read();
        push(&mut p.usage, p.now.usage);
        push(&mut p.celsius, p.now.celsius.unwrap_or(0.0));
        let watts = match &helper {
            Helper::Here { status, .. } => status.package_watts,
            Helper::Away(_) => None,
        };
        push(&mut p.watts, watts.unwrap_or(0.0));
        Frame { gpu: self.gpu.clone(), cpu: p.clone(), helper }
    }
}
