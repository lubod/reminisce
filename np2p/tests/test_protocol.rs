/// Unit tests for the np2p wire protocol (bincode serialization / framing).
/// These tests do not require a live QUIC connection — they verify that every
/// Message variant survives a round-trip through bincode and that the length
/// prefix framing never produces corrupt frames.
use np2p::network::protocol::Message;

fn roundtrip(msg: &Message) -> Message {
    let bytes = bincode::serialize(msg).expect("serialize");
    bincode::deserialize::<Message>(&bytes).expect("deserialize")
}

// ---------------------------------------------------------------------------
// Round-trip tests for every Message variant
// ---------------------------------------------------------------------------

#[test]
fn rt_handshake() {
    let msg = Message::Handshake { node_id: [0xABu8; 32], version: "0.1.0".into() };
    let rt = roundtrip(&msg);
    if let Message::Handshake { node_id, version } = rt {
        assert_eq!(node_id, [0xABu8; 32]);
        assert_eq!(version, "0.1.0");
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_handshake_ack() {
    let msg = Message::HandshakeAck { node_id: [0x11u8; 32] };
    let rt = roundtrip(&msg);
    if let Message::HandshakeAck { node_id } = rt {
        assert_eq!(node_id, [0x11u8; 32]);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_store_shard_request() {
    let payload = vec![1u8, 2, 3, 4, 5];
    let hash = [0x22u8; 32];
    let msg = Message::StoreShardRequest { shard_hash: hash, data: payload.clone() };
    let rt = roundtrip(&msg);
    if let Message::StoreShardRequest { shard_hash, data } = rt {
        assert_eq!(shard_hash, hash);
        assert_eq!(data, payload);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_store_shard_response_success() {
    let msg = Message::StoreShardResponse { shard_hash: [0xFFu8; 32], success: true };
    let rt = roundtrip(&msg);
    if let Message::StoreShardResponse { success, .. } = rt {
        assert!(success);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_retrieve_shard_request() {
    let msg = Message::RetrieveShardRequest { shard_hash: [0x33u8; 32] };
    let rt = roundtrip(&msg);
    if let Message::RetrieveShardRequest { shard_hash } = rt {
        assert_eq!(shard_hash, [0x33u8; 32]);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_retrieve_shard_response_with_data() {
    let data = vec![9u8; 256];
    let msg = Message::RetrieveShardResponse { shard_hash: [0x44u8; 32], data: Some(data.clone()) };
    let rt = roundtrip(&msg);
    if let Message::RetrieveShardResponse { data: Some(d), .. } = rt {
        assert_eq!(d, data);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_retrieve_shard_response_none() {
    let msg = Message::RetrieveShardResponse { shard_hash: [0x55u8; 32], data: None };
    let rt = roundtrip(&msg);
    if let Message::RetrieveShardResponse { data, .. } = rt {
        assert!(data.is_none());
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_heartbeat() {
    let msg = Message::Heartbeat { available_space_bytes: 123_456_789 };
    let rt = roundtrip(&msg);
    if let Message::Heartbeat { available_space_bytes } = rt {
        assert_eq!(available_space_bytes, 123_456_789);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_register_node() {
    let msg = Message::RegisterNode {
        node_id: "test-node-id".into(),
        quic_port: 5050,
        namespace: "home".into(),
    };
    let rt = roundtrip(&msg);
    if let Message::RegisterNode { node_id, quic_port, namespace } = rt {
        assert_eq!(node_id, "test-node-id");
        assert_eq!(quic_port, 5050);
        assert_eq!(namespace, "home");
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_get_peers() {
    let msg = Message::GetPeers { namespace: "home".into() };
    let rt = roundtrip(&msg);
    if let Message::GetPeers { namespace } = rt {
        assert_eq!(namespace, "home");
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_peer_list() {
    let peers = vec![
        ("node_a".to_string(), "192.168.1.1:5050".to_string()),
        ("node_b".to_string(), "192.168.1.2:5050".to_string()),
    ];
    let msg = Message::PeerList { peers: peers.clone() };
    let rt = roundtrip(&msg);
    if let Message::PeerList { peers: p } = rt {
        assert_eq!(p, peers);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_relay_request() {
    let payload = bincode::serialize(&Message::Heartbeat { available_space_bytes: 0 }).unwrap();
    let msg = Message::RelayRequest { target_node_id: "target".into(), payload: payload.clone() };
    let rt = roundtrip(&msg);
    if let Message::RelayRequest { target_node_id, payload: p } = rt {
        assert_eq!(target_node_id, "target");
        assert_eq!(p, payload);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_relay_response() {
    let payload = vec![7u8; 32];
    let msg = Message::RelayResponse { payload: payload.clone() };
    let rt = roundtrip(&msg);
    if let Message::RelayResponse { payload: p } = rt {
        assert_eq!(p, payload);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_tunnel_register() {
    let msg = Message::TunnelRegister { node_id: "home-server".into() };
    let rt = roundtrip(&msg);
    if let Message::TunnelRegister { node_id } = rt {
        assert_eq!(node_id, "home-server");
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_tunnel_challenge() {
    let nonce = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
    let msg = Message::TunnelChallenge { nonce: nonce.clone() };
    let rt = roundtrip(&msg);
    if let Message::TunnelChallenge { nonce: n } = rt {
        assert_eq!(n, nonce);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_tunnel_challenge_response() {
    let sig = vec![0xAAu8; 64];
    let msg = Message::TunnelChallengeResponse { signature: sig.clone() };
    let rt = roundtrip(&msg);
    if let Message::TunnelChallengeResponse { signature } = rt {
        assert_eq!(signature, sig);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_tunnel_accepted() {
    let msg = Message::TunnelAccepted;
    let rt = roundtrip(&msg);
    assert!(matches!(rt, Message::TunnelAccepted));
}

#[test]
fn rt_node_channel_register() {
    let msg = Message::NodeChannelRegister { node_id: "storage-node".into() };
    let rt = roundtrip(&msg);
    if let Message::NodeChannelRegister { node_id } = rt {
        assert_eq!(node_id, "storage-node");
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_node_channel_accepted() {
    let msg = Message::NodeChannelAccepted;
    let rt = roundtrip(&msg);
    assert!(matches!(rt, Message::NodeChannelAccepted));
}

#[test]
fn rt_error_message() {
    let msg = Message::Error { code: 404, message: "not found".into() };
    let rt = roundtrip(&msg);
    if let Message::Error { code, message } = rt {
        assert_eq!(code, 404);
        assert_eq!(message, "not found");
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_store_shard_stream_init() {
    let hash = [0xBBu8; 32];
    let msg = Message::StoreShardStreamInit {
        file_hash: hash,
        shard_index: 3,
        total_shard_bytes: 1_000_000,
        segment_count: 2,
    };
    let rt = roundtrip(&msg);
    if let Message::StoreShardStreamInit { file_hash, shard_index, total_shard_bytes, segment_count } = rt {
        assert_eq!(file_hash, hash);
        assert_eq!(shard_index, 3);
        assert_eq!(total_shard_bytes, 1_000_000);
        assert_eq!(segment_count, 2);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_store_shard_stream_ack() {
    let msg = Message::StoreShardStreamAck { ready: true };
    let rt = roundtrip(&msg);
    if let Message::StoreShardStreamAck { ready } = rt {
        assert!(ready);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_store_shard_chunk() {
    let data = vec![0xCCu8; 64 * 1024];
    let msg = Message::StoreShardChunk { data: data.clone() };
    let rt = roundtrip(&msg);
    if let Message::StoreShardChunk { data: d } = rt {
        assert_eq!(d.len(), 64 * 1024);
        assert_eq!(d, data);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_store_shard_stream_final() {
    let hash = [0xDDu8; 32];
    let msg = Message::StoreShardStreamFinal { shard_hash: hash };
    let rt = roundtrip(&msg);
    if let Message::StoreShardStreamFinal { shard_hash } = rt {
        assert_eq!(shard_hash, hash);
    } else { panic!("wrong variant"); }
}

#[test]
fn rt_store_shard_stream_response() {
    let msg = Message::StoreShardStreamResponse { success: false };
    let rt = roundtrip(&msg);
    if let Message::StoreShardStreamResponse { success } = rt {
        assert!(!success);
    } else { panic!("wrong variant"); }
}

// ---------------------------------------------------------------------------
// Framing: length-prefix encoding
// ---------------------------------------------------------------------------

#[test]
fn framing_length_prefix_matches_payload() {
    let msg = Message::Heartbeat { available_space_bytes: 42 };
    let payload = bincode::serialize(&msg).unwrap();
    let len = payload.len() as u32;
    let framed: Vec<u8> = len.to_be_bytes().iter().chain(payload.iter()).cloned().collect();

    let recovered_len = u32::from_be_bytes(framed[0..4].try_into().unwrap()) as usize;
    assert_eq!(recovered_len, payload.len());

    let recovered: Message = bincode::deserialize(&framed[4..4 + recovered_len]).unwrap();
    if let Message::Heartbeat { available_space_bytes } = recovered {
        assert_eq!(available_space_bytes, 42);
    } else { panic!("wrong variant after frame decode"); }
}

#[test]
fn corrupt_bytes_produce_deserialization_error() {
    let garbage = vec![0xFFu8; 32];
    let result = bincode::deserialize::<Message>(&garbage);
    assert!(result.is_err(), "random bytes should fail to deserialize as Message");
}

#[test]
fn empty_payload_produces_deserialization_error() {
    let result = bincode::deserialize::<Message>(&[]);
    assert!(result.is_err());
}
