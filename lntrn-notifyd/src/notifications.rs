use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::mpsc;
use zbus::object_server::SignalEmitter;
use zbus::{interface, zvariant::Value};

/// A notification received from D-Bus.
#[derive(Clone, Debug)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    pub timeout_ms: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

/// D-Bus interface: org.freedesktop.Notifications
pub struct NotificationService {
    next_id: Arc<AtomicU32>,
    tx: mpsc::UnboundedSender<NotifyEvent>,
}

pub enum NotifyEvent {
    Show(Notification),
    Close(u32),
}

impl NotificationService {
    pub fn new(tx: mpsc::UnboundedSender<NotifyEvent>) -> Self {
        Self {
            next_id: Arc::new(AtomicU32::new(1)),
            tx,
        }
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationService {
    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".to_string(),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        body: &str,
        _actions: Vec<String>,
        hints: HashMap<String, Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id > 0 {
            replaces_id
        } else {
            self.next_id.fetch_add(1, Ordering::Relaxed)
        };

        let urgency = match hints.get("urgency") {
            Some(Value::U8(0)) => Urgency::Low,
            Some(Value::U8(2)) => Urgency::Critical,
            _ => Urgency::Normal,
        };

        let _ = self.tx.send(NotifyEvent::Show(Notification {
            id,
            app_name: app_name.to_string(),
            summary: summary.to_string(),
            body: body.to_string(),
            urgency,
            timeout_ms: expire_timeout,
        }));

        id
    }

    fn close_notification(&self, id: u32) {
        let _ = self.tx.send(NotifyEvent::Close(id));
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "lntrn-notifyd".to_string(),
            "Lantern DE".to_string(),
            "0.1.0".to_string(),
            "1.2".to_string(), // spec version
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}
