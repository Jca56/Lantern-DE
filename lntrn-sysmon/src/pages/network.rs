//! The network: what each connection is carrying, and has carried.

use lntrn_kit::chart::{Graph, Series};
use lntrn_kit::{caption, card, note};
use lntrn_ui::Ui;

use super::{NETWORK, SECOND, at, graph, legend, peak};
use crate::format;
use crate::sample::net::Iface;
use crate::sample::{Frame, HISTORY};

/// What kind of connection it is, and how it stands.
fn link(i: &Iface) -> String {
    let kind = if i.wireless { "Wi-Fi" } else { "Wired" };
    match (i.up, i.speed) {
        (false, _) => format!("{kind}, not connected"),
        (true, Some(mbps)) if mbps >= 1000 && mbps % 1000 == 0 => format!("{kind}, {} Gb/s", mbps / 1000),
        (true, Some(mbps)) if mbps >= 1000 => format!("{kind}, {:.1} Gb/s", f64::from(mbps) / 1000.0),
        (true, Some(mbps)) => format!("{kind}, {mbps} Mb/s"),
        (true, None) => format!("{kind}, connected"),
    }
}

pub fn draw(frame: &Frame, ui: &mut Ui) {
    if frame.net.is_empty() {
        note(ui, "This machine has no network interface but its own loopback.");
    }
    for i in &frame.net {
        ui.push_id(&i.name);
        // One that is down and has carried nothing is only mentioned.
        if !i.up && i.received + i.sent == 0 {
            caption(ui, &i.name);
            card(ui, "link", |c| c.value("Link", "", &link(i)));
            ui.pop_id();
            continue;
        }
        let top = format::rate_ceiling(peak(&[&i.down_history, &i.up_history]));
        let g = Graph { series: &[Series::area(&i.down_history, NETWORK), Series::line(&i.up_history, SECOND)], max: top, slots: HISTORY, top: &format::rate(top) };
        let both = |down: f64, up: f64| format!("{} down   {} up", format::rate(down), format::rate(up));
        graph(ui, "traffic", &i.name, &both(f64::from(i.down), f64::from(i.up_rate)), frame, &g, |back| both(at(&i.down_history, back), at(&i.up_history, back)));
        legend(ui, &[(NETWORK, "Down"), (SECOND, "Up")]);
        card(ui, "link", |c| {
            c.value("Link", "", &link(i));
            c.value("Received", "Since the machine started.", &format::bytes(i.received));
            c.value("Sent", "", &format::bytes(i.sent));
        });
        ui.pop_id();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_says_its_kind_and_its_speed() {
        let wired = |up, speed| link(&Iface { up, speed, ..Iface::default() });
        assert_eq!(wired(true, Some(1000)), "Wired, 1 Gb/s");
        assert_eq!(wired(true, Some(2500)), "Wired, 2.5 Gb/s");
        assert_eq!(wired(true, Some(100)), "Wired, 100 Mb/s");
        assert_eq!(wired(false, Some(1000)), "Wired, not connected");
        assert_eq!(link(&Iface { up: true, wireless: true, ..Iface::default() }), "Wi-Fi, connected");
    }
}
