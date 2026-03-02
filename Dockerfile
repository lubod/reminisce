# Stage 1: Build with dependency caching
FROM rust:slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y libssl-dev pkg-config && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/reminisce

# 1. Copy workspace manifests only
COPY Cargo.toml Cargo.lock ./
COPY np2p/Cargo.toml ./np2p/
COPY np2p-mobile/Cargo.toml ./np2p-mobile/

# 2. Create dummy source files to cache dependency compilation
RUN mkdir -p src np2p/src/bin np2p-mobile/src && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn dummy() {}" > src/lib.rs && \
    echo "fn main() {}" > np2p/src/bin/main.rs && \
    echo "fn main() {}" > np2p/src/bin/e2e_client.rs && \
    echo "pub fn dummy() {}" > np2p/src/lib.rs && \
    echo "pub fn dummy() {}" > np2p-mobile/src/lib.rs

# Build dependencies only (this layer is cached until Cargo.toml/lock changes)
RUN cargo build --release

# 3. Copy actual source code
COPY src ./src
COPY np2p/src ./np2p/src
COPY np2p-mobile/src ./np2p-mobile/src

# Remove dummy artifacts to force re-compilation of our crates
RUN rm -f target/release/deps/reminisce* target/release/deps/libreminisce* target/release/reminisce \
         target/release/deps/np2p* target/release/deps/libnp2p*

# 4. Build the actual binary
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    ffmpeg \
    postgresql-client \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /usr/src/reminisce/target/release/reminisce /usr/local/bin/reminisce

EXPOSE 8080
EXPOSE 5050/udp

HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["reminisce"]
