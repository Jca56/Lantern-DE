//! A monitor as the page edits it, and the sums around that: what is
//! live now as drafts, what an edit asks of the compositor, the lists a
//! monitor's modes make, and what a kept setup writes to the file. The
//! drafts stand in the order of the heads they came from, one each.

use super::arrange::{self, Tile};
use crate::config::Monitor;
use crate::outputs::{Change, Head, Mode};

pub const SCALE_RANGE: (f64, f64) = (0.5, 3.0);
/// A twentieth: every step is a whole number of the 120ths windows are
/// told their scale in.
pub const SCALE_STEP: f64 = 0.05;

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub name: String,
    /// Part of the desktop, or plugged in and left out of it.
    pub enabled: bool,
    /// Which of its head's modes.
    pub mode: Option<usize>,
    pub scale: f64,
    /// Its top-left corner on the desktop, in logical pixels.
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    pub vrr: bool,
}

/// The scale a monitor is set to, from the one the compositor told us.
/// A scale crosses the wire in 256ths, so `1.4` arrives as `1.3984375`:
/// what looks exactly like a step of ours through the wire is that step.
pub fn nominal(seen: f64) -> f64 {
    let step = ((seen / SCALE_STEP).round() * SCALE_STEP * 1e6).round() / 1e6;
    if ((step * 256.0).round() / 256.0 - seen).abs() < 1e-9 { step } else { seen }
}

/// The mode `d` is set to: its own pick, else what the head is in, else
/// the head's first.
pub fn mode_of(head: &Head, d: &Draft) -> Option<Mode> {
    d.mode.or(head.current).and_then(|i| head.modes.get(i)).or(head.modes.first()).copied()
}

/// The room `d` takes on the desktop.
pub fn tile(head: &Head, d: &Draft) -> Tile {
    let (w, h) = mode_of(head, d).map_or((1920, 1080), |m| arrange::logical(m.width, m.height, d.scale));
    Tile { x: d.x, y: d.y, w, h }
}

/// What is live, as drafts: where and how the compositor has each
/// monitor, with what only the file knows (which is the main one, which
/// may vary its refresh). One that is off is where the file last had it,
/// carried into the desktop's coordinates as they are now; one the file
/// has never had is off the right end of the ones that are on, so that
/// is the side it comes on at.
pub fn base(heads: &[Head], cfg: &[Monitor]) -> Vec<Draft> {
    let entry = |name: &str| cfg.iter().find(|m| m.name == name);
    // The file's coordinates may sit off the compositor's by a constant.
    let shift = heads.iter().filter(|h| h.enabled).find_map(|h| entry(&h.name).map(|m| (h.position.0 - m.x as i32, h.position.1 - m.y as i32))).unwrap_or((0, 0));
    let mut drafts: Vec<Draft> = heads
        .iter()
        .map(|h| {
            let m = entry(&h.name);
            let (primary, vrr) = m.map_or((false, false), |m| (m.primary, m.vrr));
            if h.enabled {
                return Draft { name: h.name.clone(), enabled: true, mode: h.current, scale: nominal(h.scale), x: h.position.0, y: h.position.1, primary, vrr };
            }
            let scale = m.and_then(|m| m.scale).filter(|s| s.is_finite() && *s > 0.0).map_or(1.0, |s| (s * 1000.0).round() / 1000.0);
            let (x, y) = m.map_or((0, 0), |m| (m.x as i32 + shift.0, m.y as i32 + shift.1));
            Draft { name: h.name.clone(), enabled: false, mode: h.current, scale, x, y, primary, vrr }
        })
        .collect();
    let on: Vec<Tile> = heads.iter().zip(&drafts).filter(|(_, d)| d.enabled).map(|(h, d)| tile(h, d)).collect();
    let end = arrange::bounds(&on);
    for d in drafts.iter_mut().filter(|d| !d.enabled && entry(&d.name).is_none()) {
        (d.x, d.y) = (end.x + end.w, end.y);
    }
    drafts
}

/// Fit the monitors that are on back together after one changed size,
/// came on or went off. `was` is the drafts before that: each keeps the
/// side of its neighbour it was on. `drafts`, `was` and `heads` stand
/// side by side.
pub fn refit(drafts: &mut [Draft], heads: &[Head], was: &[Draft]) {
    let on: Vec<usize> = (0..drafts.len().min(heads.len())).filter(|&i| drafts[i].enabled).collect();
    let mut tiles: Vec<Tile> = on.iter().map(|&i| tile(&heads[i], &drafts[i])).collect();
    let before: Vec<Option<Tile>> = on.iter().map(|&i| was.get(i).filter(|w| w.enabled && w.name == drafts[i].name).map(|w| tile(&heads[i], w))).collect();
    arrange::tidy(&mut tiles, &before);
    for (&i, t) in on.iter().zip(&tiles) {
        (drafts[i].x, drafts[i].y) = (t.x, t.y);
    }
}

/// Whether going from `base` to `draft` changes anything the compositor
/// has to do, as against only what the file says.
pub fn moves_the_desktop(draft: &[Draft], base: &[Draft]) -> bool {
    draft.len() != base.len() || draft.iter().zip(base).any(|(d, b)| (d.enabled, d.mode, d.scale) != (b.enabled, b.mode, b.scale) || (d.enabled && (d.x, d.y) != (b.x, b.y)))
}

/// What to ask of the compositor to go from `base` to `draft`: every
/// monitor's place, and its mode and scale only where they change.
pub fn changes(draft: &[Draft], base: &[Draft]) -> Vec<Change> {
    draft
        .iter()
        .map(|d| {
            let b = base.iter().find(|b| b.name == d.name);
            // One coming on is told its scale whatever we think it was:
            // the compositor never said.
            let scale = (d.enabled && b.is_none_or(|b| !b.enabled || b.scale != d.scale)).then_some(d.scale);
            let mode = if d.enabled && b.is_none_or(|b| b.mode != d.mode) { d.mode } else { None };
            Change { name: d.name.clone(), enabled: d.enabled, mode, position: (d.x, d.y), scale }
        })
        .collect()
}

/// A monitor's resolutions, each once, the biggest first.
pub fn resolutions(modes: &[Mode]) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for m in modes {
        if !out.contains(&(m.width, m.height)) {
            out.push((m.width, m.height));
        }
    }
    out.sort_by_key(|&(w, h)| std::cmp::Reverse((i64::from(w) * i64::from(h), w)));
    out
}

/// The refresh rates a monitor has at `res`, each once, the fastest
/// first, with the mode it is.
pub fn rates(modes: &[Mode], res: (i32, i32)) -> Vec<(i32, usize)> {
    let mut out: Vec<(i32, usize)> = Vec::new();
    for (i, m) in modes.iter().enumerate() {
        if (m.width, m.height) == res && !out.iter().any(|(r, _)| *r == m.refresh) {
            out.push((m.refresh, i));
        }
    }
    out.sort_by_key(|&(r, _)| std::cmp::Reverse(r));
    out
}

/// The mode to go to when the resolution becomes `res`: the same refresh
/// when the monitor has it there, else the fastest.
pub fn mode_at(modes: &[Mode], res: (i32, i32), refresh: i32) -> Option<usize> {
    let rates = rates(modes, res);
    rates.iter().find(|(r, _)| *r == refresh).or(rates.first()).map(|(_, i)| *i)
}

pub fn resolution_label(modes: &[Mode], res: (i32, i32)) -> String {
    let native = modes.iter().any(|m| m.preferred && (m.width, m.height) == res);
    format!("{} × {}{}", res.0, res.1, if native { "  (native)" } else { "" })
}

/// Millihertz as hertz: `240 Hz`, `59.94 Hz`.
pub fn rate_label(millihertz: i32) -> String {
    if millihertz % 1000 == 0 { format!("{} Hz", millihertz / 1000) } else { format!("{:.2} Hz", f64::from(millihertz) / 1000.0) }
}

/// The panel's diagonal in inches, when it says how big it is.
pub fn inches(mm: (i32, i32)) -> Option<u32> {
    (mm.0 > 0 && mm.1 > 0).then(|| (f64::from(mm.0).hypot(f64::from(mm.1)) / 25.4).round() as u32)
}

/// Write a kept setup into the file's monitors. `live` is what the
/// compositor has now (what was asked for, as it took it); `wanted` is
/// what was asked for, which alone knows the main monitor and which may
/// vary their refresh. Monitors that are off or not plugged in keep
/// their place beside the others: when the ones that are on moved as a
/// whole, they move along.
pub fn keep(cfg: &mut Vec<Monitor>, heads: &[Head], live: &[Draft], wanted: &[Draft]) {
    let on = |name: &str| live.iter().find(|d| d.name == name && d.enabled);
    let shift = cfg.iter().find_map(|m| on(&m.name).map(|d| (i64::from(d.x) - m.x, i64::from(d.y) - m.y))).unwrap_or((0, 0));
    for m in cfg.iter_mut().filter(|m| on(&m.name).is_none()) {
        (m.x, m.y) = (m.x + shift.0, m.y + shift.1);
    }
    for d in live {
        let at = cfg.iter().position(|m| m.name == d.name).unwrap_or_else(|| {
            cfg.push(Monitor { name: d.name.clone(), ..Monitor::default() });
            cfg.len() - 1
        });
        let m = &mut cfg[at];
        m.enabled = d.enabled;
        if let Some(w) = wanted.iter().find(|w| w.name == d.name) {
            (m.primary, m.vrr) = (w.primary, w.vrr);
        }
        if !d.enabled {
            continue;
        }
        (m.x, m.y, m.scale) = (i64::from(d.x), i64::from(d.y), Some(d.scale));
        if let Some(mode) = heads.iter().find(|h| h.name == d.name).and_then(|h| mode_of(h, d)) {
            m.resolution = format!("{}x{}", mode.width, mode.height);
            m.refresh_rate = mode.refresh.to_string();
        }
    }
}

/// Write only what the file alone knows of `wanted`: which monitor is
/// the main one and which may vary their refresh. Nothing the compositor
/// was not asked to change is touched.
pub fn keep_flags(cfg: &mut Vec<Monitor>, wanted: &[Draft]) {
    for w in wanted {
        match cfg.iter_mut().find(|m| m.name == w.name) {
            Some(m) => (m.primary, m.vrr) = (w.primary, w.vrr),
            // No entry says nothing, which is neither.
            None if w.primary || w.vrr => cfg.push(Monitor { name: w.name.clone(), x: i64::from(w.x), y: i64::from(w.y), primary: w.primary, vrr: w.vrr, enabled: w.enabled, ..Monitor::default() }),
            None => {}
        }
    }
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
