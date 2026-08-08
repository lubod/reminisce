use np2p::network::utils::{ get_local_addrs, resolve_addr };
use np2p::error::Np2pError;

#[tokio::test]
async fn resolve_addr_parses_host_and_port() {
    let a = resolve_addr("127.0.0.1:5051").await.expect("ipv4 resolves");
    assert_eq!(a.port(), 5051);

    let b = resolve_addr("localhost:5066").await.expect("localhost resolves");
    assert_eq!(b.port(), 5066);
}

#[tokio::test]
async fn resolve_addr_rejects_garbage() {
    let e = resolve_addr("not a real address with spaces").await.unwrap_err();
    assert!(matches!(e, Np2pError::Network(_)), "got {:?}", e);

    // A host without a port fails too (invalid socket address).
    let e2 = resolve_addr("127.0.0.1").await.unwrap_err();
    assert!(matches!(e2, Np2pError::Network(_)), "got {:?}", e2);
}

#[tokio::test]
async fn get_local_addrs_returns_only_non_loopback_ipv4() {
    let addrs = get_local_addrs();
    for a in &addrs {
        assert!(!a.is_loopback(), "no loopback: {a}");
        assert!(a.is_ipv4(), "only ipv4: {a}");
    }
}
