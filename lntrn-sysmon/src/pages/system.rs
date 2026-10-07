//! What this machine is: what it runs and what is in it, with a button
//! that copies the lot as text.

use lntrn_kit::{caption, caption_action, card};
use lntrn_ui::{AreaCx, Ui};

use crate::format;
use crate::sample::Frame;

/// A card's rows: what each is, and what it says.
type Rows = Vec<(&'static str, String)>;

/// The page's rows in two runs: what the machine runs, then what is in
/// it. A fact the machine doesn't give has no row.
pub fn facts(frame: &Frame) -> (Rows, Rows) {
    let f = &frame.facts;
    let mut runs = vec![("Name", f.hostname.clone()), ("System", f.os.clone()), ("Kernel", f.kernel.clone()), ("Desktop", "Lantern, on Wayland".to_owned()), ("Shell", f.shell.clone())];
    if let Some((count, whose)) = f.packages {
        runs.push(("Packages", format!("{} ({whose})", format::count(count as u64))));
    }
    runs.push(("Up for", format::duration(frame.uptime)));

    let cores = match (f.cores, f.threads) {
        (0, 0) => String::new(),
        (0, t) => format!("{t} threads"),
        (c, t) if c == t => format!("{c} cores"),
        (c, t) => format!("{c} cores, {t} threads"),
    };
    let mut made_of = vec![("Processor", f.cpu.clone()), ("Cores", cores)];
    made_of.extend(frame.gpus.iter().map(|g| ("Graphics", g.name.clone())));
    made_of.push(("Memory", format::bytes(frame.mem.total)));
    made_of.push(("Mainboard", f.board.clone()));
    made_of.extend(f.displays.iter().map(|d| ("Display", d.clone())));
    let said = |rows: Rows| -> Rows { rows.into_iter().filter(|r| !r.1.is_empty()).collect() };
    (said(runs), said(made_of))
}

/// Every row as a line of text, for the clipboard.
pub fn summary(frame: &Frame) -> String {
    let (runs, made_of) = facts(frame);
    runs.iter().chain(&made_of).map(|(label, value)| format!("{label}: {value}\n")).collect()
}

pub fn draw(frame: &Frame, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let (runs, made_of) = facts(frame);
    if caption_action(ui, "What it runs", "Copy all") {
        ui.state.set_clipboard(summary(frame));
        cx.toast("Copied what this machine is");
    }
    for (id, title, rows) in [("runs", "", runs), ("made-of", "What is in it", made_of)] {
        if !title.is_empty() {
            caption(ui, title);
        }
        card(ui, id, |c| {
            for (label, value) in &rows {
                c.value(label, "", value);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::fixture;

    #[test]
    fn the_machine_is_summed_up_and_what_it_doesnt_say_is_left_out() {
        let text = summary(&fixture::frame());
        assert_eq!(
            text,
            "Name: testbench\nSystem: Gentoo Linux\nKernel: 7.2.9-gentoo\nDesktop: Lantern, on Wayland\nShell: zsh\nPackages: 1,203 (Portage)\nUp for: 3 d 4 h\nProcessor: Intel Core i7-14700K\nCores: 4 cores, 8 threads\nGraphics: NVIDIA GeForce RTX 3080 Ti\nMemory: 32.0 GiB\nMainboard: ASUSTeK ROG STRIX Z790-E GAMING WIFI\nDisplay: 3840×2160 (DP-1)\nDisplay: 2560×1440 (HDMI-A-1)\n"
        );
        // A machine that says next to nothing still has a page.
        let bare = summary(&fixture::bare());
        assert_eq!(bare, "Desktop: Lantern, on Wayland\nUp for: 0 s\nGraphics: Intel graphics\nMemory: 4.0 GiB\n");
    }
}
