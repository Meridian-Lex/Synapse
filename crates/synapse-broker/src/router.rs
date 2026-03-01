use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast, RwLock};

#[derive(Clone, Default)]
pub struct Router {
    channels: Arc<RwLock<HashMap<i64, broadcast::Sender<Vec<u8>>>>>,
}

impl Router {
    pub async fn subscribe(&self, channel_id: i64) -> broadcast::Receiver<Vec<u8>> {
        self.channels.write().await
            .entry(channel_id)
            .or_insert_with(|| broadcast::channel(256).0)
            .subscribe()
    }

    pub async fn publish(&self, channel_id: i64, frame: Vec<u8>) {
        if let Some(tx) = self.channels.read().await.get(&channel_id) {
            let _ = tx.send(frame);
        }
    }

    /// Prune channels with no active receivers to prevent unbounded memory growth.
    /// Call periodically (e.g. every 60s from the broker main loop).
    pub async fn prune_empty(&self) {
        self.channels.write().await
            .retain(|_, tx| tx.receiver_count() > 0);
    }
}
