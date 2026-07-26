# P2P Coordinator Daemon (`coordinator/`)

## Purpose
VPS-hosted signaling, rendezvous, and relay broker for Reminisce P2P nodes. It enables node discovery and WAN connection traversal when peers are behind symmetric NATs or firewalls.

## Architecture & Responsibilities
- **Rendezvous Signaling**: Maintains registry of active P2P nodes, public IP/port bindings, and node capabilities.
- **Relay Tunnels**: Proxies encrypted QUIC streams between nodes unable to establish direct UDP hole-punched connections.
- **Node Authentication**: Validates node Ed25519 identity key signatures during `RegisterNode` handshakes.

```
Node A (behind NAT) ──▶ Coordinator (:5055 QUIC / :8443 TCP) ──▶ Node B (behind NAT)
                                   │
                           [peers.json persistence]
```

## Default Ports
- **`:5055` (QUIC)**: Signaling, peer registration, heartbeat pings, and rendezvous lookup.
- **`:8443` (TCP)**: Reverse tunnel transport broker and fallback relay.

## State Management & Persistence
- **`peers.json`**: Active node registry serialized to disk on update.
- **Node TTL**: Registered nodes must send periodic keep-alives; nodes inactive for over 60 seconds (default `--peer-ttl-secs`) are automatically excluded from peer lists.

## Key Files
- [src/main.rs](file:///Users/ldr/work/reminisce/coordinator/src/main.rs): Standalone coordinator binary entry point, listener setup, node table, and tunnel proxying.

## Relationship to `np2p`
`coordinator` imports `np2p` protocol definitions and works directly with `np2p::network::coordinator`, `np2p::network::tunnel`, and `np2p::network::channel`.

## Invariants & Gotchas
- **Identity Enforcement**: `RegisterNode` requests MUST verify signature against the claimed Ed25519 node public key to prevent impersonation attacks in the peer registry.
