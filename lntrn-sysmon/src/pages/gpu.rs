//! The graphics cards: how busy each is, how full its memory, how hot
//! it runs and what it draws, as far as its driver will say.

use lntrn_kit::chart::{self, Graph, Series};
use lntrn_kit::{caption, card, look, note};
use lntrn_ui::Ui;

use super::{GRAPHICS, SECOND, at, graph, span};
use crate::format;
use crate::sample::gpu::Gpu;
use crate::sample::{Frame, HISTORY};

/// Where a card's temperature meter is full: about where one throttles.
const TOO_HOT: f64 = 95.0;

pub fn blurb(frame: &Frame) -> String {
    match frame.gpus.as_slice() {
        [] => "No graphics card was found.".to_owned(),
        [one] => format!("{}, on the {} driver.", one.name, one.driver),
        many => format!("{} graphics cards.", many.len()),
    }
}

pub fn draw(frame: &Frame, ui: &mut Ui) {
    if frame.gpus.is_empty() {
        note(ui, "Nothing under /sys/class/drm has a driver, and no NVIDIA driver answered.");
    }
    let several = frame.gpus.len() > 1;
    for (i, gpu) in frame.gpus.iter().enumerate() {
        ui.push_index(i);
        if several {
            caption(ui, &format!("{} ({})", gpu.name, gpu.driver));
        }
        one(frame, gpu, ui);
        ui.pop_id();
    }
}

fn one(frame: &Frame, gpu: &Gpu, ui: &mut Ui) {
    let percent = |v: f64| format::percent(v, false);
    if let Some(usage) = gpu.usage {
        let g = Graph { series: &[Series::area(&gpu.usage_history, GRAPHICS)], max: 100.0, slots: HISTORY, top: "100%" };
        graph(ui, "usage", &format!("In use, {}", span(frame)), &percent(f64::from(usage)), frame, &g, |back| percent(at(&gpu.usage_history, back)));
    }
    if let Some((used, total)) = gpu.memory {
        let g = Graph { series: &[Series::area(&gpu.memory_history, SECOND)], max: 100.0, slots: HISTORY, top: "100%" };
        graph(ui, "memory", &format!("Video memory, {}", span(frame)), &format::bytes_of(used, total), frame, &g, |back| percent(at(&gpu.memory_history, back)));
    }

    let quiet = gpu.celsius.is_none() && gpu.watts.is_none() && gpu.fans.is_empty() && gpu.core_mhz.is_none() && gpu.memory_mhz.is_none();
    if !quiet {
        caption(ui, "How it is running");
        card(ui, "running", |c| {
            if let Some(celsius) = gpu.celsius {
                let frac = f64::from(celsius) / TOO_HOT;
                c.meter("Temperature", "", frac, &format::celsius(f64::from(celsius)), chart::heat(frac, GRAPHICS));
            }
            match (gpu.watts, gpu.watts_limit) {
                (Some(watts), Some(limit)) => {
                    let frac = f64::from(watts / limit);
                    c.meter("Power", "What it draws, of the most it is allowed.", frac, &format!("{watts:.0} of {limit:.0} W"), chart::heat(frac, GRAPHICS));
                }
                (Some(watts), None) => c.value("Power", "", &format!("{watts:.0} W")),
                _ => {}
            }
            for (n, fan) in gpu.fans.iter().enumerate() {
                let label = if gpu.fans.len() > 1 { format!("Fan {}", n + 1) } else { "Fan".to_owned() };
                let said = if *fan == 0 { "Stopped".to_owned() } else { format!("{fan}%") };
                c.meter(&label, "", f64::from(*fan) / 100.0, &said, look::TEXT_DIM);
            }
            if let Some(mhz) = gpu.core_mhz {
                c.value("Core clock", "", &format::clock(f64::from(mhz)));
            }
            if let Some(mhz) = gpu.memory_mhz {
                c.value("Memory clock", "", &format::clock(f64::from(mhz)));
            }
        });
    }
    if gpu.usage.is_none() && gpu.memory.is_none() {
        note(ui, &format!("The {} driver doesn't say how busy this card is or how much of its memory is in use.", gpu.driver));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::fixture;

    #[test]
    fn the_blurb_names_one_card_and_counts_several() {
        let mut f = fixture::frame();
        assert_eq!(blurb(&f), "NVIDIA GeForce RTX 3080 Ti, on the nvidia 615.71.09 driver.");
        f.gpus.push(Gpu::default());
        assert_eq!(blurb(&f), "2 graphics cards.");
        f.gpus.clear();
        assert_eq!(blurb(&f), "No graphics card was found.");
    }
}
