mod filechooser;
mod request;

use filechooser::FileChooserService;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    eprintln!("[lntrn-portal] starting...");

    let service = FileChooserService::new();
    let conn = zbus::connection::Builder::session()?
        .name("org.freedesktop.impl.portal.desktop.lantern")?
        .serve_at("/org/freedesktop/impl/portal", service)?
        .build()
        .await?;

    eprintln!("[lntrn-portal] registered on D-Bus session bus");

    // Store connection for dynamic Request object registration
    filechooser::set_connection(conn);

    // Keep alive forever — zbus processes D-Bus messages in the background
    std::future::pending::<()>().await;
    Ok(())
}
