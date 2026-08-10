use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use tokio::sync::broadcast;

const USER_EVENT_CAPACITY: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderEvent {
    version: u8,
    kind: ReaderEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    feed_id: Option<String>,
}

impl ReaderEvent {
    #[must_use]
    pub const fn sync_required() -> Self {
        Self {
            version: 1,
            kind: ReaderEventKind::SyncRequired,
            feed_id: None,
        }
    }

    #[must_use]
    pub fn feed_refreshed(feed_id: impl Into<String>) -> Self {
        Self {
            version: 1,
            kind: ReaderEventKind::FeedRefreshed,
            feed_id: Some(feed_id.into()),
        }
    }

    #[must_use]
    pub const fn subscriptions_changed() -> Self {
        Self {
            version: 1,
            kind: ReaderEventKind::SubscriptionsChanged,
            feed_id: None,
        }
    }

    #[must_use]
    pub const fn entries_changed() -> Self {
        Self {
            version: 1,
            kind: ReaderEventKind::EntriesChanged,
            feed_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ReaderEventKind {
    SyncRequired,
    FeedRefreshed,
    SubscriptionsChanged,
    EntriesChanged,
}

#[derive(Clone, Default)]
pub struct ReaderEventHub {
    users: Arc<Mutex<HashMap<String, broadcast::Sender<ReaderEvent>>>>,
}

impl ReaderEventHub {
    #[must_use]
    pub fn subscribe(&self, user_id: &str) -> ReaderEventSubscription {
        let mut users = self
            .users
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sender = users
            .entry(user_id.to_owned())
            .or_insert_with(|| broadcast::channel(USER_EVENT_CAPACITY).0);
        ReaderEventSubscription {
            user_id: user_id.to_owned(),
            receiver: sender.subscribe(),
            users: Arc::clone(&self.users),
        }
    }

    pub fn publish(&self, user_id: &str, event: ReaderEvent) {
        let mut users = self
            .users
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(sender) = users.get(user_id) else {
            return;
        };
        if sender.receiver_count() == 0 {
            users.remove(user_id);
            return;
        }
        let _ = sender.send(event);
    }
}

pub struct ReaderEventSubscription {
    user_id: String,
    receiver: broadcast::Receiver<ReaderEvent>,
    users: Arc<Mutex<HashMap<String, broadcast::Sender<ReaderEvent>>>>,
}

impl ReaderEventSubscription {
    pub async fn recv(&mut self) -> ReaderEventReceive {
        match self.receiver.recv().await {
            Ok(event) => ReaderEventReceive::Event(event),
            Err(broadcast::error::RecvError::Lagged(_)) => ReaderEventReceive::Lagged,
            Err(broadcast::error::RecvError::Closed) => ReaderEventReceive::Closed,
        }
    }
}

impl Drop for ReaderEventSubscription {
    fn drop(&mut self) {
        let mut users = self
            .users
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if users
            .get(&self.user_id)
            .is_some_and(|sender| sender.receiver_count() <= 1)
        {
            users.remove(&self.user_id);
        }
    }
}

pub enum ReaderEventReceive {
    Event(ReaderEvent),
    Lagged,
    Closed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_are_isolated_by_user_and_unused_channels_are_removed() {
        let hub = ReaderEventHub::default();
        let mut first = hub.subscribe("user-a");
        let mut second = hub.subscribe("user-b");

        hub.publish("user-a", ReaderEvent::entries_changed());
        assert!(matches!(first.recv().await, ReaderEventReceive::Event(_)));
        assert!(second.receiver.try_recv().is_err());

        drop(first);
        assert!(!hub.users.lock().unwrap().contains_key("user-a"));
        drop(second);
        assert!(hub.users.lock().unwrap().is_empty());
    }

    #[test]
    fn event_contract_is_versioned_and_uses_discriminated_kinds() {
        assert_eq!(
            serde_json::to_value(ReaderEvent::feed_refreshed("feed-1")).unwrap(),
            serde_json::json!({
                "version": 1,
                "kind": "FEED_REFRESHED",
                "feedId": "feed-1",
            }),
        );
        assert_eq!(
            serde_json::to_value(ReaderEvent::sync_required()).unwrap(),
            serde_json::json!({ "version": 1, "kind": "SYNC_REQUIRED" }),
        );
    }
}
