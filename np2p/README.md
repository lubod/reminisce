# `np2p` Crate

## Purpose
A lightweight, secure peer-to-peer networking and distributed backup library written in Rust. Used by Reminisce for node-to-node shard replication, LAN discovery, and WAN connectivity via QUIC tunnels.

## Module Map
```
np2p/
├── src/
│   ├── crypto/     # Ed25519 node identity, mTLS cert generation, ShardToken signing
│   ├── network/    # QUIC transport, message framing, LAN UDP discovery, coordinator relay
│   └── storage/    # ChaCha20-Poly1305 encryption, Reed-Solomon erasure coding (3/5), disk I/O
├── bin/
│   └── np2pd.rs    # Standalone P2P storage node daemon
└── DESIGN.md       # High-level architecture & protocol goals
```

## Key Specs & Protocol Constants
- **`PROTOCOL_VERSION`**: `0.2.0` — checked on every handshake; mismatched peers are rejected (the streaming `StoreShardStreamInit` message gained a `ShardToken` in 0.2.0, changing the wire format).
- **Node Identity**: Ed25519 keypair formatted as hex string Node ID.
- **Server identity binding**: Every `Node::connect` verifies the peer's self-signed cert public key equals the dialed 64-hex Node ID (split into two DNS labels, `first32.second32`, because a single 64-char label exceeds the 63-char DNS limit). Connections to non-node-id names are refused.
- **Coordinator node ID**: Clients dialing the coordinator must supply its 64-hex Node ID (`--coordinator-node-id`; `coordinator_node_id` config for the backend). Without it, coordinator/tunnel use is refused.
- **Streaming upload auth**: `StoreShardStreamInit` carries a `ShardToken` bound to `blake3(file_hash || shard_index)`; storage nodes verify it before accepting chunks and enforce a 1 GiB per-shard cap plus a free-space guard.
- **Detailed Design**: Refer to [DESIGN.md](file:///Users/ldr/work/reminisce/np2p/DESIGN.md) for complete protocol specification.

## Subdirectory Documentation
- [src/network/](file:///Users/ldr/work/reminisce/np2p/src/network/README.md): Network transport, discovery, and message framing.
- [src/storage/](file:///Users/ldr/work/reminisce/np2p/src/storage/README.md): Shard encryption, erasure coding, and disk layout.

## Invariants & Gotchas
- **Protocol Version Alignment**: All nodes participating in P2P mesh or connecting to coordinator must share the exact same `PROTOCOL_VERSION`.
- **Authorized owner pin**: Storage nodes should set `--authorized-node-id` so both direct and relayed (channel) requests are owner-pinned; the relay path no longer bypasses the pin.
- **`sni_for_node_id`**: `Node::connect` rejects non-64-hex names — never pass a literal like `"reminisce"` again.
