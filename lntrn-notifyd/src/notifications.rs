use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use zbus::object_server::SignalEmitter;
use zbus::{interface, zvariant::Value};

/// A notification received from D-Bus.
#[derive(Clone, Debug)]
pub struct Notification {
    pub id: u32,
    #[allow(dead_code)]
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    pub timeout_ms: i32,
    /// Its actions: each one's key, and its label.
    pub actions: Vec<(String, String)>,
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

/// What became of a notification, for the app that sent it: an action
/// clicked, or it closed (why: 1 it expired, 2 it was dismissed, 3 a
/// `CloseNotification` closed it).
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Action(u32, String),
    Closed(u32, u32),
}

/// The spec's `actions`, key then label, as pairs; an odd one out is
/// dropped.
pub fn pairs(actions: Vec<String>) -> Vec<(String, String)> {
    actions.chunks_exact(2).map(|p| (p[0].clone(), p[1].clone())).collect()
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
        vec!["actions".to_string(), "body".to_string()]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
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
            actions: pairs(actions),
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
    pub async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_come_in_pairs() {
        let said = |a: &[&str]| pairs(a.iter().map(|s| s.to_string()).collect());
        assert_eq!(said(&["default", "Open", "folder", "Show in folder"]), [("default".into(), "Open".into()), ("folder".into(), "Show in folder".into())]);
        assert_eq!(said(&["open", "Open", "stray"]).len(), 1, "an odd one out is dropped");
    }
}
