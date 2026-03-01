# np2p Design Document

## 1. Objective
Build a lightweight, secure, and mobile-friendly Peer-to-Peer (P2P) networking library. It serves two primary functions:
1.  **Transport:** Secure, NAT-traversing tunnel between mobile and home server.
2.  **Storage:** Distributed, resilient backup using Erasure Coding across storage nodes.

## 2. Core Requirements

### Connectivity & Discovery
- **UDP Hole Punching:** Primary method for NAT traversal.
- **Relay Fallback:** Lightweight TURN-like relay for symmetric NATs.
- **Local Discovery:** mDNS/broadcast for finding peers on the same LAN.
- **Global Discovery:** A lightweight "Rendezvous Server" for IP:Port exchange.

### Security
- **Identity:** Ed25519 Public Keys as Node IDs.
- **Transport Security:** AEAD (ChaCha20-Poly1305) with Noise Protocol handshake.
- **Privacy:** Metadata encryption (don't reveal filenames or user IDs to the relay).

### Distributed Storage (The Backup System)
- **Erasure Coding (EC):** 3/5 Reed-Solomon scheme.
    - Data is split into 3 data shards and 2 parity shards (total 5).
    - Any 3 shards are sufficient to reconstruct the original file.
- **Encryption-at-Rest:**
    - Files are encrypted with a per-file key *before* sharding.
    - Shards are individually signed to prevent tampering by storage nodes.
- **Content Addressing:** BLAKE3 hashes for file and shard identification.
- **Shard Distribution:** Protocol for discovering "Storage-capable" peers and negotiating storage space.
- **Health Auditing:** Periodic checks to ensure storage nodes still have the shards they promised to keep.

### Transport Layer
- **Protocol:** UDP with a reliability and congestion control layer (QUIC-based).
- **Multiplexing:** Support concurrent control and data streams.

### Mobile Specifics
- **Roaming:** Connection migration (IP changes don't drop the session).
- **Resource Management:** Battery-efficient keep-alives.

## 3. Architecture Overview

### Components
1.  **Node:** The local peer (Mobile, Home Server, or Storage Node).
2.  **Rendezvous Server:** Signaling for NAT traversal.
3.  **Storage Registry:** (Part of the Home Server) Tracks which shards are on which nodes.
4.  **Storage Engine:** Logic for EC encoding/decoding and shard integrity verification.

## 4. Storage Flow
1.  **Ingest:** Home Server receives file from Mobile.
2.  **Encrypt:** Home Server encrypts file with a master key.
3.  **Shard:** Split encrypted file into 5 shards (3 data + 2 parity).
4.  **Distribute:** Send shards to 5 different storage nodes via `np2p` tunnels.
5.  **Track:** Update local database with shard locations.

## 5. Roadmap
1.  **Transport Foundation:** Reliable UDP / QUIC integration.
2.  **Signaling:** Rendezvous server implementation.
3.  **Erasure Coding:** Integrate `reed-solomon-erasure` crate.
4.  **Distribution Protocol:** Logic for pushing/pulling shards to/from peers.
5.  **Auditing:** Implement shard integrity "heartbeats".
