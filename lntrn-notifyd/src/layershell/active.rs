//! A notification on screen: when it came, how long it shows, how far it
//! has slid in or out, its actions, and why it went (for the app that
//! sent it).

use std::time::Instant;

use crate::notifications::{Notification, Urgency};
use crate::render::{Hit, Toast};

const SLIDE_IN_SECS: f32 = 0.5;
const SLIDE_OUT_SECS: f32 = 0.4;
const CRITICAL_DISPLAY_SECS: f32 = 10.0;
/// Buttons a toast shows, at most.
const BUTTONS: usize = 3;

/// Why a notification closed, as the spec numbers it.
pub const EXPIRED: u32 = 1;
pub const DISMISSED: u32 = 2;
pub const CLOSED_BY_CALL: u32 = 3;

pub struct ActiveNotification {
    title: String,
    body: String,
    urgency: Urgency,
    spawned: Instant,
    display_secs: f32,
    pub id: u32,
    /// Its actions, key and label; "default" is a click anywhere on it.
    actions: Vec<(String, String)>,
    /// Why it's going, once it is going early.
    pub reason: Option<u32>,
    /// What the pointer is over.
    pub hover: Option<Hit>,
}

impl ActiveNotification {
    pub fn from_notification(notif: &Notification, default_secs: f32) -> Self {
        let display = if notif.urgency == Urgency::Critical {
            CRITICAL_DISPLAY_SECS
        } else if notif.timeout_ms > 0 {
            (notif.timeout_ms as f32 / 1000.0).clamp(2.0, 30.0)
        } else {
            default_secs
        };

        Self {
            title: notif.summary.clone(),
            body: notif.body.clone(),
            urgency: notif.urgency,
            spawned: Instant::now(),
            display_secs: display,
            id: notif.id,
            actions: notif.actions.clone(),
            reason: None,
            hover: None,
        }
    }

    /// The keys of the actions that show as buttons, in order.
    fn buttons(&self) -> impl Iterator<Item = &(String, String)> {
        self.actions.iter().filter(|(key, _)| key != "default").take(BUTTONS)
    }

    /// The key of the action a click on `hit` invokes.
    pub fn action_of(&self, hit: Hit) -> Option<String> {
        match hit {
            Hit::Close => None,
            Hit::Action(n) => self.buttons().nth(n).map(|(key, _)| key.clone()),
            Hit::Body => self.actions.iter().any(|(key, _)| key == "default").then(|| "default".to_owned()),
        }
    }

    pub fn to_toast(&self) -> Toast {
        Toast {
            title: self.title.clone(),
            body: self.body.clone(),
            urgency: self.urgency,
            progress: self.progress(),
            slide: self.slide(),
            buttons: self.buttons().map(|(_, label)| label.clone()).collect(),
            default: self.actions.iter().any(|(key, _)| key == "default"),
            hover: self.hover,
        }
    }

    fn elapsed(&self) -> f32 {
        self.spawned.elapsed().as_secs_f32()
    }

    fn progress(&self) -> f32 {
        let elapsed = self.elapsed();
        (1.0 - elapsed / self.display_secs).clamp(0.0, 1.0)
    }

    /// 0.0 = off-screen, 1.0 = fully visible
    fn slide(&self) -> f32 {
        let elapsed = self.elapsed();
        if elapsed < SLIDE_IN_SECS {
            // Sliding in
            elapsed / SLIDE_IN_SECS
        } else if elapsed >= self.display_secs {
            // Sliding out
            let out_elapsed = elapsed - self.display_secs;
            (1.0 - out_elapsed / SLIDE_OUT_SECS).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    pub fn is_expired(&self) -> bool {
        self.elapsed() >= self.display_secs + SLIDE_OUT_SECS
    }

    /// Trigger an early slide-out, as `reason` says. Pulls `spawned` back
    /// so `elapsed` already equals `display_secs`, putting the toast at
    /// the start of its slide-out animation.
    pub fn dismiss(&mut self, reason: u32) {
        self.reason.get_or_insert(reason);
        if self.elapsed() < self.display_secs {
            self.spawned = Instant::now() - std::time::Duration::from_secs_f32(self.display_secs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(actions: &[(&str, &str)]) -> ActiveNotification {
        let notif = Notification {
            id: 4,
            app_name: "Bluetooth".into(),
            summary: "Received photo.jpg".into(),
            body: String::new(),
            urgency: Urgency::Normal,
            timeout_ms: -1,
            actions: actions.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect(),
        };
        ActiveNotification::from_notification(&notif, 5.0)
    }

    #[test]
    fn a_default_action_is_a_click_on_it_and_the_rest_are_buttons() {
        let a = with(&[("default", "Open"), ("open", "Open"), ("folder", "Show in folder")]);
        let toast = a.to_toast();
        assert_eq!((toast.buttons, toast.default), (vec!["Open".to_owned(), "Show in folder".to_owned()], true));
        assert_eq!(a.action_of(Hit::Action(1)).as_deref(), Some("folder"));
        assert_eq!(a.action_of(Hit::Body).as_deref(), Some("default"));
        assert_eq!(a.action_of(Hit::Close), None);
        let plain = with(&[]);
        assert_eq!((plain.to_toast().default, plain.action_of(Hit::Body)), (false, None));
    }

    #[test]
    fn going_early_keeps_the_first_reason() {
        let mut a = with(&[]);
        a.dismiss(DISMISSED);
        a.dismiss(CLOSED_BY_CALL);
        assert_eq!(a.reason, Some(DISMISSED));
    }
}
