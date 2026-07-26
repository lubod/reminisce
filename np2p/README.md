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
- **`PROTOCOL_VERSION`**: Fixed protocol version handshake check across all connecting peers.
- **Node Identity**: Ed25519 keypair formatted as hex string Node ID.
- **Detailed Design**: Refer to [DESIGN.md](file:///Users/ldr/work/reminisce/np2p/DESIGN.md) for complete protocol specification.

## Subdirectory Documentation
- [src/network/](file:///Users/ldr/work/reminisce/np2p/src/network/README.md): Network transport, discovery, and message framing.
- [src/storage/](file:///Users/ldr/work/reminisce/np2p/src/storage/README.md): Shard encryption, erasure coding, and disk layout.

## Invariants & Gotchas
- **Protocol Version Alignment**: All nodes participating in P2P mesh or connecting to coordinator must share the exact same `PROTOCOL_VERSION`.
