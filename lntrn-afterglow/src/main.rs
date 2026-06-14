//! lntrn-afterglow GUI — live hardware monitoring (tuning controls land next).
//!
//! Runs unprivileged and reads telemetry from lntrn-afterglowd over its Unix
//! socket; the root daemon owns every hardware write.

mod gui;

fn main() {
    gui::run();
}
