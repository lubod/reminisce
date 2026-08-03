use serde::{Serialize, Deserialize};
use crate::crypto::ShardToken;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Message {
    /// Initial handshake to identify peers and negotiate versions.
    Handshake { node_id: [u8; 32], version: String },
    HandshakeAck { node_id: [u8; 32] },

    /// Request to store an encrypted shard.
    StoreShardRequest { shard_hash: [u8; 32], data: Vec<u8>, token: ShardToken },
    StoreShardResponse { shard_hash: [u8; 32], success: bool },

    /// Request to retrieve an encrypted shard.
    RetrieveShardRequest { shard_hash: [u8; 32], token: ShardToken },
    RetrieveShardResponse { shard_hash: [u8; 32], data: Option<Vec<u8>> },

    /// Periodic heartbeat to maintain NAT mappings and report status.
    Heartbeat { available_space_bytes: u64 },

    /// Coordinator: register this node. Coordinator replies with PeerList.
    RegisterNode { node_id: String, quic_port: u16, namespace: String },

    /// Coordinator: request the current peer list. Coordinator replies with PeerList.
    GetPeers { namespace: String },

    /// Coordinator: list of known peers as ("node_id", "ip:port") pairs.
    PeerList { peers: Vec<(String, String)> },

    /// Relay: forward payload (bincode-serialized Message) to target node.
    /// Coordinator replies with RelayResponse or Error.
    RelayRequest { target_node_id: String, payload: Vec<u8> },

    /// Relay: the response bytes (bincode-serialized Message) from the target.
    RelayResponse { payload: Vec<u8> },

    /// Tunnel: home server registers itself as the HTTP tunnel backend (step 1).
    TunnelRegister { node_id: String },

    /// Tunnel: coordinator sends a random nonce for the home server to sign (step 2).
    TunnelChallenge { nonce: Vec<u8> },

    /// Tunnel: home server replies with Ed25519 signature over the nonce (step 3).
    TunnelChallengeResponse { signature: Vec<u8> },

    /// Tunnel: coordinator acknowledges the tunnel registration (step 4).
    TunnelAccepted,

    /// Channel: storage node registers a persistent reverse channel for relay (step 1).
    NodeChannelRegister { node_id: String },

    /// Channel: coordinator sends nonce to sign (step 2).
    NodeChannelChallenge { nonce: Vec<u8> },

    /// Channel: storage node replies with Ed25519 signature (step 3).
    NodeChannelChallengeResponse { signature: Vec<u8> },

    /// Channel: coordinator accepts the channel registration (step 4).
    NodeChannelAccepted,

    /// Generic error message.
    Error { code: u16, message: String },

    /// Streaming shard upload for large files: one stream per shard, followed by
    /// StoreShardChunk messages (≤ 32 MB each), terminated by StoreShardStreamFinal.
    /// The token is created over blake3(file_hash || shard_index) and is verified by
    /// the storage node before any chunk is accepted (mirrors StoreShardRequest auth).
    StoreShardStreamInit {
        file_hash: [u8; 32],
        shard_index: u8,
        total_shard_bytes: u64,
        segment_count: u32,
        token: ShardToken,
    },
    StoreShardStreamAck { ready: bool },
    StoreShardChunk { data: Vec<u8> },
    StoreShardStreamFinal { shard_hash: [u8; 32] },
    StoreShardStreamResponse { success: bool },

    /// Request to delete a shard (e.g. retention pruning of old DB snapshots).
    /// Appended last to preserve bincode wire indices of all earlier variants.
    DeleteShardRequest { shard_hash: [u8; 32], token: ShardToken },
    DeleteShardResponse { shard_hash: [u8; 32], success: bool },

    /// Name-addressed, overwritable object store for small critical metadata
    /// (e.g. the DB-backup restore manifest) so it survives a home-server disk loss.
    /// Token is created over blake3(name). Appended last for wire compatibility.
    StorePinnedObject { name: String, data: Vec<u8>, token: ShardToken },
    StorePinnedResponse { success: bool },
    GetPinnedObject { name: String, token: ShardToken },
    PinnedObjectResponse { data: Option<Vec<u8>> },
}

pub struct Protocol;

impl Protocol {
    pub async fn send(send: &mut quinn::SendStream, msg: &Message) -> crate::error::Result<()> {
        let bytes = bincode::serialize(msg)?;
        let len = bytes.len() as u32;

        send.write_all(&len.to_be_bytes()).await?;
        send.write_all(&bytes).await?;
        Ok(())
    }

    pub async fn receive(recv: &mut quinn::RecvStream) -> crate::error::Result<Message> {
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        // Cap must accommodate the largest intentional message we can receive:
        // a relayed shard payload (RetrieveShardResponse) for the largest single
        // shard (~85 MB for a 256 MB file split into 3 data shards) plus bincode
        // framing overhead. Peers in a mesh are identity-authenticated, so this is
        // a trust-boundary limit (an unpinned storage node is already an open relay).
        if len > 128 * 1024 * 1024 {
            return Err(crate::error::Np2pError::Protocol("Message too large".into()));
        }

        let mut buf = vec![0u8; len];
        recv.read_exact(&mut buf).await?;

        let msg: Message = bincode::deserialize(&buf).map_err(|e| {
            // Distinguish unknown enum variant (version mismatch) from genuine corruption.
            if e.to_string().contains("expected a variant index") {
                crate::error::Np2pError::UnknownMessage(e.to_string())
            } else {
                crate::error::Np2pError::Serialization(e)
            }
        })?;
        Ok(msg)
    }
}
