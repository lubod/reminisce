# Security

This document summarises the security posture of Reminisce and the hardening that has
been applied. It covers authentication/authorization, the P2P crypto boundary, and the
operational defaults you should not undo.

## Threat model (summary)

- The web/API layer assumes an attacker who may be an **unauthenticated network peer**
  or a **low-privilege logged-in user** (e.g. a malicious `viewer`/`user`), including
  attempts to escalate via the short-lived media token.
- The P2P layer assumes **any internet host can reach storage nodes and the
  coordinator**, and that the coordinator relay is **untrusted for confidentiality**
  (it sees relayed payloads). Authenticity is enforced, secrecy is not via the relay.
- An attacker with the **master secret** (`api_secret_key`) can forge the system, so it
  must be protected (see [deployment.md](deployment.md)).

## Web / API authentication

- Sessions use an **httpOnly, Secure, SameSite=Lax cookie**. JWTs are kept **out of
  localStorage** and are never placed in URLs.
- Two parallel auth paths exist but now share one rule set:
  - `Claims` extractor (cookie/session) and `authenticate_request` (Bearer / cookie)
    both re-read the **role from the database on every request** (5s cache) rather than
    trusting the JWT's baked-in role.
  - **Token scopes are enforced everywhere.** The short-lived 24h `image_token`
    (`media_read` scope) is only accepted for raw media byte-serving
    (`get_image`, `get_video`); every other endpoint (delete/star/restore/import/
    labels/persons/admin) returns **403** for it.
- The `?token=` query-parameter auth path has been **removed** (dead legacy that leaked
  JWTs into logs/referrers).
- HTTP request tracing logs **path only** (no query string), so parameters never reach
  logs/OTLP.
- **Rate limiting** buckets by the real client IP: proxy headers (`X-Forwarded-For`,
  `X-Real-IP`) are only trusted when the direct peer is loopback/private, and the
  proxy-appended (last) XFF entry is used — a client cannot rotate its IP header to
  bypass the login brute-force limit.
- Server-side directory import is **admin-only** and its filesystem walk never follows
  symlinks (containment cannot be escaped).

## Transport (nginx)

- TLS 1.2/1.3, AEAD-only ciphers, HSTS + security headers on the HTTPS front.
- Credential-bearing paths (`/api/auth/user-login-form`, `/api/auth/user-login`,
  `/api/auth/setup`) are **307-redirected to HTTPS** on the HTTP listener, so raw
  passwords never POST in cleartext. Media/health stay available on HTTP for LAN/
  Android tooling. **Deployments exposing the API to the public Internet must enforce
  HTTPS end-to-end.**

## P2P / storage (`np2p` + `coordinator`)

- **Identity = Ed25519 key, bound to TLS.** Each node's public key is its node_id; QUIC
  connections pin the certificate to the dialed node_id (`sni_for_node_id` + cert
  verifier). Public keys are extracted from certificates with a **strict
  subjectPublicKeyInfo structure walk** (not a DER byte-pattern scan), closing the
  crafted-cert identity-impersonation (MITM / registry poisoning) primitive.
- **Shard tokens are operation-bound.** A token signs
  `op_tag ‖ shard_hash ‖ timestamp` with `ShardOp::{Store, Retrieve, Delete}`, so a
  captured retrieve token can never be replayed to delete (or store) the same shard.
  The 5-minute timestamp window bounds replay.
- **Storage nodes fail closed.** `np2pd` refuses to start unless the owner is pinned
  with `--authorized-node-id <home-server-node-id>` **or** the operator explicitly
  passes `--allow-unpinned`. Running unpinned (or owner-pinned) restricts who may
  store/retrieve/delete shards.
- **Encryption nonce safety.** The ChaCha20-Poly1305 nonce is derived from
  `blake3(key ‖ context ‖ content_hash)`, so two distinct payloads can never reuse a
  nonce under the same key+context (identical content still reproduces identical
  ciphertext for shard-repair compatibility).
- Coordinator: relay reads are time-bounded (30s), and channel registration removal on
  disconnect is guarded by the connection's `stable_id` (a stale disconnect task cannot
  wipe a reconnecting node).
- Known gap: the coordinator relay transits plaintext payloads (authenticity-preserving,
  confidentiality-neutral). Sensitive shards are encrypted before upload.

## Operations / deployment defaults

- `generate_install.sh` generates a **random 64-hex `api_secret_key`** at install time
  and no longer ships a default admin password. There is **no pre-seeded admin** — the
  first user creates it via the setup screen.
- **Migrations fail fast.** A failed versioned migration is never recorded as applied
  and the server exits instead of starting on an inconsistent schema.
- **Observability is private.** Grafana requires login (anonymous-admin disabled) and is
  bound to `127.0.0.1` along with Prometheus/Alloy; external networks cannot reach them
  or inject metrics. Loki log retention is **7 days**.
- Dev DB/AI ports bind to loopback (tests/backend still reach them via localhost).
- The deploy pipeline runs a **secret scan** over tracked files (JWTs, private keys,
  `AKIA*`, placeholder secrets) and refuses to ship if any are found.

## Known gaps / follow-ups

- **Android TLS**: the app disables certificate/hostname validation for private/local
  server addresses and allows cleartext for its NetBird/CGNAT HTTP flow. This is a
  deliberate product trade-off; the recommended follow-up is HTTPS-first with
  per-host user-confirmed HTTP opt-in and TOFU pinning. Needs a build environment to
  validate.
- Docker images are not digest-pinned.
- `NodeIdentity::from_secret` uses a single BLAKE3 (no memory-hard KDF); fine for the
  high-entropy 64-hex secret it hashes, weaker for low-entropy inputs.

## Reporting

Self-hosted project — no formal channel. Please open an issue with affected version and
reproducer.
