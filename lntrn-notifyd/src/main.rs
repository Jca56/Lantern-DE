mod layershell;
mod notifications;

use notifications::{NotificationService, NotifyEvent};
use tokio::sync::mpsc;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<NotifyEvent>();

    // Spawn Wayland render loop on a dedicated thread
    let render_handle = std::thread::spawn(move || {
        if let Err(e) = layershell::run(rx) {
            eprintln!("lntrn-notifyd render error: {e}");
        }
    });

    // Claim the notification daemon name on D-Bus session bus
    let service = NotificationService::new(tx);
    let _conn = zbus::connection::Builder::session()?
        .name("org.freedesktop.Notifications")?
        .serve_at("/org/freedesktop/Notifications", service)?
        .build()
        .await?;

    // Keep tokio runtime alive (D-Bus messages are processed in the background)
    // by waiting for the render thread on a blocking task
    tokio::task::spawn_blocking(move || {
        render_handle.join().ok();
    })
    .await?;

    Ok(())
}
