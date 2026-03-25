// Ring buffer with sequence numbering and FIFO eviction
use crate::types::{BufferedMessage, MessageType};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub struct ChannelBuffer {
    capacity: usize,
    messages: VecDeque<BufferedMessage>,
    next_seq: u64,
}

impl ChannelBuffer {
    /// Create a new ring buffer with the given capacity. Panics if capacity is zero.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "ChannelBuffer capacity must be > 0");
        Self {
            capacity,
            messages: VecDeque::new(),
            next_seq: 1,
        }
    }

    /// Internal push with all metadata fields.
    fn push(
        &mut self,
        msg_type: MessageType,
        text: Option<String>,
        payload: Option<serde_json::Value>,
    ) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let received_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if self.messages.len() >= self.capacity {
            self.messages.pop_front(); // FIFO eviction
        }

        self.messages.push_back(BufferedMessage {
            seq,
            msg_type,
            text,
            payload,
            received_at,
        });
    }

    /// Push a dialogue message.
    pub fn push_dialogue(&mut self, text: String) {
        self.push(MessageType::Dialogue, Some(text), None);
    }

    /// Push a work message.
    pub fn push_work(&mut self, value: serde_json::Value) {
        self.push(MessageType::Work, None, Some(value));
    }

    /// Return all messages with seq > since. Does not consume them.
    pub fn drain_since(&self, since: u64) -> Vec<BufferedMessage> {
        self.messages
            .iter()
            .filter(|m| m.seq > since)
            .cloned()
            .collect()
    }

    /// Return the next sequence number that will be assigned.
    #[allow(dead_code)]
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Return the number of messages currently buffered.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Return true if the buffer is empty.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Long-poll: wait until at least `min` messages with seq > since arrive,
    /// or `timeout_ms` elapses. Returns whatever was collected.
    pub async fn wait_for(
        buf: Arc<Mutex<Self>>,
        since: u64,
        min: usize,
        timeout_ms: u64,
    ) -> Vec<BufferedMessage> {
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            {
                let guard = buf.lock().expect("buffer lock poisoned");
                let msgs = guard.drain_since(since);
                if msgs.len() >= min {
                    return msgs;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        buf.lock().expect("buffer lock poisoned").drain_since(since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_push_and_drain() {
        let mut buf = ChannelBuffer::new(10);
        buf.push_dialogue("hello".into());
        let msgs = buf.drain_since(0);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_ref().unwrap(), "hello");
    }

    #[test]
    fn test_seq_increments() {
        let mut buf = ChannelBuffer::new(10);
        buf.push_dialogue("a".into());
        buf.push_dialogue("b".into());
        buf.push_dialogue("c".into());
        let msgs = buf.drain_since(0);
        assert_eq!(msgs[0].seq, 1);
        assert_eq!(msgs[1].seq, 2);
        assert_eq!(msgs[2].seq, 3);
    }
}
