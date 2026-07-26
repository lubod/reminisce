# Distributed Storage & Encryption (`np2p/src/storage/`)

## Purpose
Handles file encryption, Reed-Solomon erasure coding, content-addressed disk storage, and shard integrity verification.

## Storage Pipeline
```
Raw Data Stream ──▶ ChaCha20-Poly1305 Encrypt ──▶ Reed-Solomon (3 Data + 2 Parity) ──▶ Shard Disk Layout
```

1. **Encryption (`encryption.rs`)**: Data encrypted using ChaCha20-Poly1305 AEAD cipher.
2. **Erasure Coding (`erasure.rs`)**: Encrypted file split into 5 shards (3 data shards + 2 parity shards). Reconstruction requires any 3 out of 5 shards.
3. **Disk Engine (`disk.rs`)**: Shards saved using content-addressed paths with atomic file swaps.

## Disk Storage Layout
Shards are stored under the storage root directory partitioned by the first two hex characters of their BLAKE3 hash:
```
<storage_dir>/shards/<hash[0..2]>/<hash[2..]>
```

## Key Files
- [encryption.rs](file:///Users/ldr/work/reminisce/np2p/src/storage/encryption.rs): ChaCha20-Poly1305 cipher implementation.
- [erasure.rs](file:///Users/ldr/work/reminisce/np2p/src/storage/erasure.rs): 3/5 Reed-Solomon encoder and decoder.
- [disk.rs](file:///Users/ldr/work/reminisce/np2p/src/storage/disk.rs): Content-addressed filesystem read/write, disk quota, and space checks.

## Critical Invariants & Gotchas
- **Deterministic Nonce Invariant**: ChaCha20-Poly1305 nonces MUST be generated deterministically using `blake3(file_hash + chunk_index)`. **NEVER generate random nonces**. Deterministic nonces are required so independent nodes and audit workers can verify and repair missing shards without needing shared state.
- **Atomic Disk Writes**: Always write incoming shards to `<path>.tmp` first, then atomically rename to `<path>` upon byte completion and BLAKE3 verification.
- **Storage Space Safety Guard**: `disk.rs` MUST verify that the target filesystem has at least 100 MB free space available before accepting or writing new shards.
