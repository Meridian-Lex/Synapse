use synapse_relay::buffer::ChannelBuffer;
use synapse_relay::config::Config;
use synapse_relay::relay::RelayRegistry;
use std::sync::{Arc, Mutex};

/// Verify that messages pushed directly to the buffer are retrievable.
/// (Integration with relay.rs requires live TLS infrastructure — tested manually.)
#[tokio::test]
async fn test_buffer_poll_integration() {
    let buf = Arc::new(Mutex::new(ChannelBuffer::new(100)));
    let buf2 = buf.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        buf2.lock().unwrap().push_dialogue("hello from mock broker".into());
    });
    let msgs = ChannelBuffer::wait_for(buf, 0, 1, 500).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].text.as_deref(), Some("hello from mock broker"));
}

#[test]
fn test_config_defaults_for_relay() {
    let cfg = Config::defaults();
    assert_eq!(cfg.broker_host, "localhost");
    assert_eq!(cfg.broker_port, 7777);
    assert_eq!(cfg.buffer_capacity, 1000);
    assert_eq!(cfg.reconnect_delay_secs, 2);
    assert_eq!(cfg.reconnect_max_secs, 30);
}

#[test]
fn test_relay_registry_new() {
    let cfg = Config {
        agent_name: "test".into(),
        secret: "test".into(),
        ..Default::default()
    };
    let registry = RelayRegistry::new(cfg);
    let status = registry.status();
    assert!(status.is_empty());
}

#[test]
fn test_relay_poll_unsubscribed_returns_empty() {
    let cfg = Config {
        agent_name: "test".into(),
        secret: "test".into(),
        ..Default::default()
    };
    let registry = RelayRegistry::new(cfg);
    let msgs = registry.poll("#nonexistent", 0);
    assert!(msgs.is_empty());
}
