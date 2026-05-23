//! Event bus for helper-emitted asynchronous notifications.
//!
//! Wraps `tokio::sync::broadcast`. Multiple subscribers can `subscribe()` to
//! get an independent stream; if a subscriber falls behind the broadcast
//! channel will drop events for that subscriber (logged as a warning) but
//! never block the read loop. The default channel depth is sized for typical
//! foreground / AX event rates (a few per second peak); callers that need
//! lossless delivery should drain the receiver promptly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::protocol::Event;

use super::protocol::event_names;

/// Public typed event surface. Each new phase adds variants here for the
/// events it emits; unknown event names land in [`HelperEvent::Unknown`]
/// so a future helper can advertise new events without breaking older
/// clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HelperEvent {
    /// Phase 3 — emitted after the foreground app changes.
    ForegroundAppActivated {
        ts: DateTime<Utc>,
        pid: Option<i32>,
        bundle_id: Option<String>,
        app_name: Option<String>,
        window_title: Option<String>,
    },

    /// Phase 3 — focused-window change inside the currently subscribed pid.
    AxFocusedWindowChanged {
        ts: DateTime<Utc>,
        pid: i32,
        bundle_id: Option<String>,
        window_title: Option<String>,
    },

    /// Phase 3 — focused window's title text changed.
    AxTitleChanged {
        ts: DateTime<Utc>,
        pid: i32,
        bundle_id: Option<String>,
        window_title: Option<String>,
    },

    /// Phase 5 — a recording segment file finished writing.
    RecordingSegmentClosed {
        ts: DateTime<Utc>,
        session_id: String,
        segment_index: u32,
        path: String,
        duration_ms: u64,
        sys_track_present: bool,
        mic_track_present: bool,
    },

    /// Phase 5 — non-fatal recording warning or fatal stop.
    RecordingError {
        ts: DateTime<Utc>,
        session_id: String,
        code: String,
        message: String,
        fatal: bool,
    },

    /// Catch-all for events whose `name` doesn't match a known typed
    /// variant.
    Unknown {
        name: String,
        ts: DateTime<Utc>,
        payload: Option<serde_json::Value>,
    },
}

impl HelperEvent {
    pub(super) fn from_wire(event: Event) -> Self {
        let payload = event.payload.clone().unwrap_or(serde_json::Value::Null);
        match event.name.as_str() {
            event_names::FOREGROUND_APP_ACTIVATED => HelperEvent::ForegroundAppActivated {
                ts: event.ts,
                pid: payload
                    .get("pid")
                    .and_then(|v| v.as_i64())
                    .map(|n| n as i32),
                bundle_id: payload
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                app_name: payload
                    .get("app_name")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                window_title: payload
                    .get("window_title")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            },
            event_names::AX_FOCUSED_WINDOW_CHANGED => HelperEvent::AxFocusedWindowChanged {
                ts: event.ts,
                pid: payload
                    .get("pid")
                    .and_then(|v| v.as_i64())
                    .map(|n| n as i32)
                    .unwrap_or(0),
                bundle_id: payload
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                window_title: payload
                    .get("window_title")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            },
            event_names::AX_TITLE_CHANGED => HelperEvent::AxTitleChanged {
                ts: event.ts,
                pid: payload
                    .get("pid")
                    .and_then(|v| v.as_i64())
                    .map(|n| n as i32)
                    .unwrap_or(0),
                bundle_id: payload
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                window_title: payload
                    .get("window_title")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            },
            event_names::RECORDING_SEGMENT_CLOSED => HelperEvent::RecordingSegmentClosed {
                ts: event.ts,
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_default(),
                segment_index: payload
                    .get("segment_index")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .unwrap_or(0),
                path: payload
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_default(),
                duration_ms: payload
                    .get("duration_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                sys_track_present: payload
                    .get("sys_track_present")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                mic_track_present: payload
                    .get("mic_track_present")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            },
            event_names::RECORDING_ERROR => HelperEvent::RecordingError {
                ts: event.ts,
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_default(),
                code: payload
                    .get("code")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| "UNKNOWN".into()),
                message: payload
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_default(),
                fatal: payload
                    .get("fatal")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            },
            _ => HelperEvent::Unknown {
                name: event.name,
                ts: event.ts,
                payload: event.payload,
            },
        }
    }
}

/// Default event-bus capacity. 1024 chosen so a 1-second backpressure
/// window at typical AX/foreground event rates (< 100/s) leaves headroom.
pub const DEFAULT_EVENT_CAPACITY: usize = 1024;

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<HelperEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_EVENT_CAPACITY);
        Self { sender }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Subscribe to all helper events. Subscribers that lag will see
    /// `RecvError::Lagged(n)` from the receiver — the channel drops the
    /// oldest events for them but keeps the global stream flowing.
    pub fn subscribe(&self) -> broadcast::Receiver<HelperEvent> {
        self.sender.subscribe()
    }

    /// Publish an event. No-op if there are no subscribers.
    pub fn publish(&self, event: HelperEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[tokio::test]
    async fn subscribers_receive_published_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.publish(HelperEvent::Unknown {
            name: "foreground.app_activated".into(),
            ts: Utc::now(),
            payload: Some(json!({"pid": 1234})),
        });
        let evt = rx.recv().await.unwrap();
        match evt {
            HelperEvent::Unknown { name, .. } => assert_eq!(name, "foreground.app_activated"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn from_wire_with_unknown_name_yields_unknown_variant() {
        let wire = Event {
            name: "some.future.event".into(),
            ts: Utc::now(),
            payload: None,
        };
        let typed = HelperEvent::from_wire(wire);
        match typed {
            HelperEvent::Unknown { name, .. } => assert_eq!(name, "some.future.event"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn from_wire_maps_foreground_app_activated_to_typed_variant() {
        let wire = Event {
            name: "foreground.app_activated".into(),
            ts: Utc::now(),
            payload: Some(json!({
                "pid": 4321,
                "bundle_id": "com.example",
                "app_name": "Example",
                "window_title": "Hello",
            })),
        };
        match HelperEvent::from_wire(wire) {
            HelperEvent::ForegroundAppActivated {
                pid,
                bundle_id,
                app_name,
                window_title,
                ..
            } => {
                assert_eq!(pid, Some(4321));
                assert_eq!(bundle_id.as_deref(), Some("com.example"));
                assert_eq!(app_name.as_deref(), Some("Example"));
                assert_eq!(window_title.as_deref(), Some("Hello"));
            }
            other => panic!("expected ForegroundAppActivated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn from_wire_maps_ax_focused_window_changed_to_typed_variant() {
        let wire = Event {
            name: "ax.focused_window_changed".into(),
            ts: Utc::now(),
            payload: Some(json!({
                "pid": 99,
                "window_title": "New Doc",
            })),
        };
        match HelperEvent::from_wire(wire) {
            HelperEvent::AxFocusedWindowChanged {
                pid, window_title, ..
            } => {
                assert_eq!(pid, 99);
                assert_eq!(window_title.as_deref(), Some("New Doc"));
            }
            other => panic!("expected AxFocusedWindowChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_subscribers_means_silent_publish() {
        let bus = EventBus::new();
        // No panic, no error: publish is fire-and-forget.
        bus.publish(HelperEvent::Unknown {
            name: "x".into(),
            ts: Utc::now(),
            payload: None,
        });
    }
}
