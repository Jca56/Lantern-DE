//! Storage: how full each filesystem is, and how hard each drive is
//! being read and written.

use lntrn_kit::chart::{self, Graph, Series};
use lntrn_kit::{caption, card, note};
use lntrn_ui::Ui;

use super::{SECOND, STORAGE, at, graph, legend, peak};
use crate::format;
use crate::sample::disk::{Drive, Volume};
use crate::sample::{Frame, HISTORY};

/// What a filesystem's row says under its mount point: the device it
/// is on, its kind, and where else it is mounted.
fn about(v: &Volume) -> String {
    let device = v.device.strip_prefix("/dev/").unwrap_or(&v.device);
    match v.also.as_slice() {
        [] => format!("{device}, {}", v.kind),
        also => format!("{device}, {}. Also at {}.", v.kind, also.join(", ")),
    }
}

/// What a drive is called over its graph: what it calls itself, where
/// it says, and always the kernel's name for it.
fn title(d: &Drive) -> String {
    let size = format::bytes(d.size);
    if d.model.is_empty() { format!("{}, {size}", d.name) } else { format!("{} ({}), {size}", d.model, d.name) }
}

pub fn draw(frame: &Frame, ui: &mut Ui) {
    let disks = &frame.disks;
    caption(ui, "Filesystems");
    if disks.volumes.is_empty() {
        note(ui, "Nothing on a drive is mounted.");
    } else {
        card(ui, "volumes", |c| {
            for v in &disks.volumes {
                let frac = v.used() as f64 / v.total.max(1) as f64;
                c.meter(&v.mount, &about(v), frac, &format::bytes_of(v.used(), v.total), chart::heat(frac, STORAGE));
            }
        });
    }

    for d in &disks.drives {
        // Both lines on one scale, which grows to fit the busier.
        let top = format::rate_ceiling(peak(&[&d.read_history, &d.write_history]));
        let g = Graph { series: &[Series::area(&d.read_history, STORAGE), Series::line(&d.write_history, SECOND)], max: top, slots: HISTORY, top: &format::rate(top) };
        let both = |read: f64, written: f64| format!("{} read   {} written", format::rate(read), format::rate(written));
        graph(ui, &d.name, &title(d), &both(f64::from(d.read), f64::from(d.written)), frame, &g, |back| both(at(&d.read_history, back), at(&d.write_history, back)));
        legend(ui, &[(STORAGE, "Read"), (SECOND, "Written")]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::fixture;

    #[test]
    fn a_filesystem_and_a_drive_say_what_they_are() {
        let f = fixture::frame();
        assert_eq!(about(&f.disks.volumes[0]), "nvme0n1p2, btrfs. Also at /home, /.snapshots.");
        assert_eq!(about(&f.disks.volumes[1]), "sda1, btrfs");
        assert_eq!(title(&f.disks.drives[0]), "Samsung SSD 990 PRO 1TB (nvme0n1), 931 GiB");
        assert_eq!(title(&f.disks.drives[1]), "sda, 3.6 TiB");
    }
}
