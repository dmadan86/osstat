//! What the window does when it is closed.
//!
//! The decision is held here, in Rust, rather than fetched from the webview
//! when the close arrives: `CloseRequested` must be answered synchronously
//! inside the event handler, and there is no opportunity to ask anything.
//!
//! That also makes the behaviour robust in the case that matters. A
//! confirmation dialog would have to be drawn by the webview that is about to
//! disappear; if it ever failed to draw, the close button would silently do
//! nothing and osstat would be dismissible only through its tray icon. Reading
//! a value already in memory cannot get stuck that way.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// What closing the window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-bindings", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts-bindings", ts(export))]
#[serde(rename_all = "camelCase")]
pub enum CloseBehaviour {
    /// Keep running in the notification area.
    #[default]
    Hide,
    /// Exit.
    Quit,
}

impl CloseBehaviour {
    /// Whether closing should hide the window instead of exiting.
    #[must_use]
    pub const fn hides(self) -> bool {
        matches!(self, Self::Hide)
    }
}

/// The current close behaviour, held as Tauri managed state.
#[derive(Debug, Default)]
pub struct CloseSetting(Mutex<CloseBehaviour>);

impl CloseSetting {
    /// The current behaviour.
    #[must_use]
    pub fn get(&self) -> CloseBehaviour {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Replaces the behaviour.
    pub fn set(&self, behaviour: CloseBehaviour) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = behaviour;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn hiding_is_the_default() {
        assert_eq!(CloseBehaviour::default(), CloseBehaviour::Hide);
        assert_eq!(CloseSetting::default().get(), CloseBehaviour::Hide);
    }

    #[test]
    fn a_change_survives_a_round_trip() {
        let setting = CloseSetting::default();
        setting.set(CloseBehaviour::Quit);
        assert_eq!(setting.get(), CloseBehaviour::Quit);
    }

    #[test]
    fn serialises_as_the_strings_the_front_end_stores() {
        assert_eq!(
            serde_json::to_string(&CloseBehaviour::Hide).unwrap(),
            "\"hide\""
        );
        assert_eq!(
            serde_json::to_string(&CloseBehaviour::Quit).unwrap(),
            "\"quit\""
        );
    }

    #[test]
    fn only_hiding_keeps_the_app_running() {
        assert!(CloseBehaviour::Hide.hides());
        assert!(!CloseBehaviour::Quit.hides());
    }
}
