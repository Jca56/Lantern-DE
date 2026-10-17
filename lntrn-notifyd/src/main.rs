mod layershell;
mod notifications;
mod render;

use notifications::{NotificationService, NotifyEvent, Outcome};
use tokio::sync::mpsc;

const PATH: &str = "/org/freedesktop/Notifications";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let (tx, rx) = mpsc::unbounded_channel::<NotifyEvent>();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Outcome>();

    // Spawn Wayland render loop on a dedicated thread
    let render_handle = std::thread::spawn(move || {
        if let Err(e) = layershell::run(rx, out_tx) {
            eprintln!("lntrn-notifyd render error: {e}");
        }
    });

    // Claim the notification daemon name on D-Bus session bus
    let service = NotificationService::new(tx);
    let conn = zbus::connection::Builder::session()?
        .name("org.freedesktop.Notifications")?
        .serve_at(PATH, service)?
        .build()
        .await?;

    // What became of each notification, told to the apps as the spec's
    // signals: an action clicked, it closed and why.
    let iface = conn.object_server().interface::<_, NotificationService>(PATH).await?;
    tokio::spawn(async move {
        while let Some(outcome) = out_rx.recv().await {
            let emitter = iface.signal_emitter();
            let sent = match outcome {
                Outcome::Action(id, key) => NotificationService::action_invoked(emitter, id, &key).await,
                Outcome::Closed(id, reason) => NotificationService::notification_closed(emitter, id, reason).await,
            };
            if let Err(e) = sent {
                eprintln!("lntrn-notifyd: can't say what became of a notification: {e}");
            }
        }
    });

    // Keep tokio runtime alive (D-Bus messages are processed in the background)
    // by waiting for the render thread on a blocking task
    tokio::task::spawn_blocking(move || {
        render_handle.join().ok();
    })
    .await?;

    Ok(())
}
