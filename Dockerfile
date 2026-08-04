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
RUN apt-get update && apt-get install -y libssl-dev pkg-config mold clang protobuf-compiler && rm -rf /var/lib/apt/lists/*

# Build dependencies with cache mounts for cargo registry and target
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo chef cook --release --recipe-path recipe.json

# Stage 3: Builder - Build the actual application
FROM rust:slim-bookworm AS builder
WORKDIR /app
# Install build dependencies including mold linker
RUN apt-get update && apt-get install -y libssl-dev pkg-config mold clang protobuf-compiler && rm -rf /var/lib/apt/lists/*

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

# PostgreSQL 16 client (matches the server; Debian bookworm ships v15 which cannot
# pg_dump a v16 server). PGDG repo provides postgresql-client-16.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg \
    && install -d /usr/share/postgresql-common/pgdg \
    && curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc -o /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc \
    && echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] https://apt.postgresql.org/pub/repos/apt bookworm-pgdg main" > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        ffmpeg \
        postgresql-client-16 \
    && rm -rf /var/lib/apt/lists/*

# Create a non-privileged user and group
RUN groupadd -g 10001 reminisce && \
    useradd -u 10001 -g reminisce -m -s /sbin/nologin reminisce

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/bin/reminisce /usr/local/bin/reminisce

# Set permissions
RUN chown -R reminisce:reminisce /app

EXPOSE 8080
EXPOSE 5050/udp

# Switch to the non-privileged user
USER reminisce:reminisce

HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["reminisce"]
