# P2P Network Layer (`np2p/src/network/`)

## Purpose
Manages QUIC transport connections, peer discovery on local networks, coordinator signaling relays, message framing, and bi-directional stream multiplexing.

## Framing & Wire Protocol
- **Message Framing**: `[4-byte Big-Endian Length Header] + [Bincode-serialized Payload]`.
- **Payload Cap**: Max message payload size is capped at 20 MB (`MAX_FRAME_SIZE`) to prevent memory exhaustion from malformed streams.
- **Transport**: Quinn (QUIC over UDP). TLS certificates are dynamically derived from node Ed25519 identity keys.
- **Shard lifecycle**: `StoreShard*` (upload), `RetrieveShard*` (download), and `DeleteShard*` (retention pruning). New `Message` variants MUST be appended at the end of the enum — bincode encodes the variant index, so inserting/reordering breaks wire compatibility with existing peers.
- **Pinned objects**: `StorePinnedObject` / `GetPinnedObject` provide a name-addressed, overwritable store for small critical metadata (e.g. the DB-backup restore manifest), token-authed over `blake3(name)`. This lets the restore map live on the mesh and survive a home-server disk loss.

## Discovery & Routing Topology
```
Local LAN ──▶ UDP Broadcast (:5066) ──▶ Signed Announcement (90s TTL) ──▶ Direct QUIC Mesh
                                                                                  │
WAN Fallback ──▶ Coordinator (VPS) ──▶ Reverse Tunnel / QUIC Relay ───────────────┘
```

1. **LAN Discovery (`discovery.rs`)**: Nodes broadcast signed UDP packets on port 5066 every 10 seconds. Peers maintain local registry entries with 90-second TTLs.
2. **Direct Connection**: Primary mode connects directly to target node IP via QUIC.
3. **Coordinator Relay (`coordinator.rs`, `tunnel.rs`)**: If direct connection fails (NAT/firewall), nodes establish reverse tunnels through the VPS Coordinator.
4. **Channels (`channel.rs`)**: High-level bi-directional streaming abstraction over raw QUIC streams.

## Key Files
- [protocol.rs](file:///Users/ldr/work/reminisce/np2p/src/network/protocol.rs): Message definitions and enum payloads.
- [transport.rs](file:///Users/ldr/work/reminisce/np2p/src/network/transport.rs): Quinn QUIC endpoint creation and TLS configuration.
- [discovery.rs](file:///Users/ldr/work/reminisce/np2p/src/network/discovery.rs): UDP LAN broadcast discovery.
- [tunnel.rs](file:///Users/ldr/work/reminisce/np2p/src/network/tunnel.rs): Reverse tunnel handling over coordinator relay.

## Invariants & Gotchas
- **Streaming Shard Upload Hash Check**: When receiving streaming shard uploads, nodes compute BLAKE3 hashes incrementally on incoming bytes and write to `.tmp` files. Shards are finalized and renamed ONLY if the calculated BLAKE3 hash matches the expected shard ID.
- **Relay Fallback**: Direct connection attempts MUST time out before falling back to relay tunnels to prioritize LAN zero-latency paths.
