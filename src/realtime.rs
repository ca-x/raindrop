use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use tokio::sync::{broadcast, watch};

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
    shutdown_tx: watch::Sender<bool>,
}

impl ReaderEventHub {
    #[must_use]
    pub fn subscribe(&self, user_id: &str) -> ReaderEventSubscription {
        let mut users = self.users.lock().expect("reader event hub lock poisoned");
        // A subscription may have deferred cleanup while another request held this lock.
        users.retain(|_, sender| sender.receiver_count() > 0);
        let sender = users
            .entry(user_id.to_owned())
            .or_insert_with(|| broadcast::channel(USER_EVENT_CAPACITY).0);
        ReaderEventSubscription {
            user_id: user_id.to_owned(),
            receiver: sender.subscribe(),
            users: Arc::clone(&self.users),
            shutdown_rx: self.shutdown_tx.subscribe(),
        }
    }

    /// Closes current and future event subscriptions so HTTP connections can drain.
    pub fn shutdown(&self) {
        self.shutdown_tx.send_replace(true);
    }

    pub fn publish(&self, user_id: &str, event: ReaderEvent) {
        let mut users = self.users.lock().expect("reader event hub lock poisoned");
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
    shutdown_rx: watch::Receiver<bool>,
}

impl ReaderEventSubscription {
    pub async fn recv(&mut self) -> ReaderEventReceive {
        let result = tokio::select! {
            biased;
            _ = self.shutdown_rx.wait_for(|shutdown| *shutdown) => return ReaderEventReceive::Closed,
            result = self.receiver.recv() => result,
        };
        match result {
            Ok(event) => ReaderEventReceive::Event(event),
            Err(broadcast::error::RecvError::Lagged(_)) => ReaderEventReceive::Lagged,
            Err(broadcast::error::RecvError::Closed) => ReaderEventReceive::Closed,
        }
    }
}

impl Drop for ReaderEventSubscription {
    fn drop(&mut self) {
        // Teardown must not block or panic, including during unwinding after a poisoned lock.
        let Ok(mut users) = self.users.try_lock() else {
            return;
        };
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
    async fn shutdown_closes_current_and_future_subscriptions_even_with_buffered_events() {
        let hub = ReaderEventHub::default();
        let existing = hub.subscribe("reader");
        hub.publish("reader", ReaderEvent::entries_changed());
        hub.shutdown();

        for mut subscription in [existing, hub.subscribe("reader")] {
            let received =
                tokio::time::timeout(std::time::Duration::from_secs(1), subscription.recv())
                    .await
                    .expect("event subscriptions must finish promptly after shutdown");
            assert!(matches!(received, ReaderEventReceive::Closed));
        }
    }

    #[test]
    fn dropping_a_subscription_does_not_wait_for_the_hub_lock() {
        let hub = ReaderEventHub::default();
        let subscription = hub.subscribe("departing-reader");
        let users = hub.users.lock().expect("fresh event hub lock");
        let (dropped_tx, dropped_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            drop(subscription);
            dropped_tx.send(()).expect("drop observer remains alive");
        });
        let dropped_without_lock = dropped_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_ok();
        drop(users);
        worker.join().expect("subscription drop must not panic");
        assert!(
            dropped_without_lock,
            "subscription teardown must not wait for another request's event hub lock"
        );

        let _next = hub.subscribe("next-reader");
        assert!(
            !hub.users
                .lock()
                .expect("event hub lock")
                .contains_key("departing-reader"),
            "the next subscription must prune deferred channel cleanup"
        );
    }

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
