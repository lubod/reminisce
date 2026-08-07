use bytes::Bytes;
use serial_test::serial;

use reminisce::services::proxy_manager::ProxyManager;

#[actix_web::test]
#[serial]
async fn proxy_stream_routing() {
    let pm = ProxyManager::new();

    // Register a stream and receive its body chunks.
    let mut rx = pm.register("req-1".to_string()).await;

    pm.push_chunk("req-1", Bytes::from_static(b"hello ")).await.expect("push 1");
    pm.push_chunk("req-1", Bytes::from_static(b"world")).await.expect("push 2");

    let first = rx.recv().await.expect("chunk 1");
    let second = rx.recv().await.expect("chunk 2");
    assert_eq!(&first[..], b"hello ");
    assert_eq!(&second[..], b"world");

    // Removing the stream closes the receiver (no more chunks).
    pm.remove("req-1").await;
    assert!(rx.recv().await.is_none(), "receiver closes after remove");
}

#[actix_web::test]
#[serial]
async fn proxy_push_to_unknown_stream_fails() {
    let pm = ProxyManager::new();
    assert!(pm.push_chunk("never-registered", Bytes::from_static(b"x")).await.is_err());
}
