use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Message {
    /// Initial handshake to identify peers and negotiate versions.
    Handshake { node_id: [u8; 32], version: String },
    HandshakeAck { node_id: [u8; 32] },

    /// Token-based authentication (Mobile -> Home Server).
    Authenticate { token: String },
    AuthenticateResponse { success: bool, message: String },

    /// Password-based login (Mobile -> Home Server via P2P).
    LoginRequest {
        username: String,
        password_hash: String,
    },
    LoginResponse {
        success: bool,
        token: Option<String>,
        message: String,
    },

    /// Request to store an encrypted shard.
    StoreShardRequest { shard_hash: [u8; 32], data: Vec<u8> },
    StoreShardResponse { shard_hash: [u8; 32], success: bool },

    /// Request to retrieve an encrypted shard.
    RetrieveShardRequest { shard_hash: [u8; 32] },
    RetrieveShardResponse { shard_hash: [u8; 32], data: Option<Vec<u8>> },

    /// Request to upload a full media file (Mobile -> Home Server).
    UploadMediaRequest {
        device_id: String,
        file_hash: String,
        file_name: String,
        file_ext: String,
        data: Vec<u8>,
    },
    UploadMediaResponse { success: bool, message: String },

    /// Periodic heartbeat to maintain NAT mappings and report status.
    Heartbeat { available_space_bytes: u64 },

    /// Generic error message.
    Error { code: u16, message: String },
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

        if len > 100 * 1024 * 1024 { // 100MB limit
            return Err(crate::error::Np2pError::Protocol("Message too large".into()));
        }

        let mut buf = vec![0u8; len];
        recv.read_exact(&mut buf).await?;
        
        let msg: Message = bincode::deserialize(&buf)?;
        Ok(msg)
    }
}
