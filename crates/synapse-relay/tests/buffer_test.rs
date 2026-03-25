use synapse_relay::buffer::ChannelBuffer;

#[test]
fn test_seq_starts_at_one() {
    let mut buf = ChannelBuffer::new(10);
    buf.push_dialogue("hello".into());
    let msgs = buf.drain_since(0);
    assert_eq!(msgs[0].seq, 1);
}

#[test]
fn test_drain_since_filters() {
    let mut buf = ChannelBuffer::new(10);
    buf.push_dialogue("a".into());
    buf.push_dialogue("b".into());
    buf.push_dialogue("c".into());
    let msgs = buf.drain_since(2);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].seq, 3);
}

#[test]
fn test_fifo_eviction_at_capacity() {
    let mut buf = ChannelBuffer::new(3);
    buf.push_dialogue("a".into()); // seq 1
    buf.push_dialogue("b".into()); // seq 2
    buf.push_dialogue("c".into()); // seq 3
    buf.push_dialogue("d".into()); // seq 4 — evicts seq 1
    let msgs = buf.drain_since(0);
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].seq, 2); // oldest is now seq 2
    assert_eq!(msgs[2].seq, 4);
}

#[test]
fn test_next_seq_after_eviction() {
    let mut buf = ChannelBuffer::new(2);
    buf.push_dialogue("a".into());
    buf.push_dialogue("b".into());
    buf.push_dialogue("c".into());
    assert_eq!(buf.next_seq(), 4);
}

#[tokio::test]
async fn test_wait_for_resolves_on_arrival() {
    use std::sync::{Arc, Mutex};
    let buf = Arc::new(Mutex::new(ChannelBuffer::new(100)));
    let buf2 = buf.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        buf2.lock().unwrap().push_dialogue("arrived".into());
    });
    let msgs = ChannelBuffer::wait_for(buf, 0, 1, 500).await;
    assert_eq!(msgs.len(), 1);
}

#[tokio::test]
async fn test_wait_for_times_out() {
    use std::sync::{Arc, Mutex};
    let buf = Arc::new(Mutex::new(ChannelBuffer::new(100)));
    let msgs = ChannelBuffer::wait_for(buf, 0, 1, 50).await;
    assert!(msgs.is_empty());
}
