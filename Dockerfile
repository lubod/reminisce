# Stage 1: Planner - Generate recipe for dependencies
FROM rust:slim-bookworm AS planner
WORKDIR /app
RUN cargo install cargo-chef 
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Cacher - Build dependencies only
FROM rust:slim-bookworm AS cacher
WORKDIR /app
RUN cargo install cargo-chef
COPY --from=planner /app/recipe.json recipe.json
# Install build dependencies including mold linker
RUN apt-get update && apt-get install -y libssl-dev pkg-config mold clang && rm -rf /var/lib/apt/lists/*

# Build dependencies with cache mounts for cargo registry and target
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo chef cook --release --recipe-path recipe.json

# Stage 3: Builder - Build the actual application
FROM rust:slim-bookworm AS builder
WORKDIR /app
# Install build dependencies including mold linker
RUN apt-get update && apt-get install -y libssl-dev pkg-config mold clang && rm -rf /var/lib/apt/lists/*

# Copy source
COPY . .

# Final build with cache mounts. Copy binary out of mount after build.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    mkdir -p /app/bin && \
    cp target/release/reminisce /app/bin/reminisce

# Stage 4: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    ffmpeg \
    postgresql-client \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/bin/reminisce /usr/local/bin/reminisce

EXPOSE 8080
EXPOSE 5050/udp

HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["reminisce"]
