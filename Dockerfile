# Stage 1: Planner - Generate recipe for dependencies
FROM rust:slim-bookworm@sha256:96c0af8cf054fd006435089f0076729716784ec9be485bd655de59c55df105ce AS planner
WORKDIR /app
RUN cargo install cargo-chef
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: Cacher - Build dependencies only. The compiled dependency tree is
# committed to this layer (no /app/target cache mount), so BuildKit layer cache
# reuses it whenever the recipe (Cargo.toml/lock) and compiler are unchanged.
FROM rust:slim-bookworm@sha256:96c0af8cf054fd006435089f0076729716784ec9be485bd655de59c55df105ce AS cacher
WORKDIR /app
RUN cargo install cargo-chef
COPY --from=planner /app/recipe.json recipe.json
# Install build dependencies including mold linker
RUN apt-get update && apt-get install -y libssl-dev pkg-config mold clang protobuf-compiler && rm -rf /var/lib/apt/lists/*

# Keep compiler parallelism capped so the shared host/container stays responsive.
ARG CARGO_BUILD_JOBS=24
# Cache mount only for the cargo registry to avoid re-downloading crates.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef cook --release -j ${CARGO_BUILD_JOBS} --recipe-path recipe.json

# Stage 3: Builder - Build the actual application on top of the cacher's deps
FROM rust:slim-bookworm@sha256:96c0af8cf054fd006435089f0076729716784ec9be485bd655de59c55df105ce AS builder
WORKDIR /app
# Install build dependencies including mold linker
RUN apt-get update && apt-get install -y libssl-dev pkg-config mold clang protobuf-compiler && rm -rf /var/lib/apt/lists/*

# Reuse the precompiled dependency artifacts from the cacher (layer-cached), so a
# source change only recompiles the workspace crates instead of all ~490 deps.
COPY --from=cacher /app/target /app/target

# Copy source
COPY . .

ARG CARGO_BUILD_JOBS=24
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release -j ${CARGO_BUILD_JOBS} && \
    mkdir -p /app/bin && \
    cp target/release/reminisce /app/bin/reminisce && \
    cp target/release/disaster_recovery /app/bin/disaster_recovery && \
    cp target/release/p2p_restore /app/bin/p2p_restore

# Stage 4: Runtime
FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241

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
COPY --from=builder /app/bin/disaster_recovery /usr/local/bin/disaster_recovery
COPY --from=builder /app/bin/p2p_restore /usr/local/bin/p2p_restore

# Set permissions
RUN chown -R reminisce:reminisce /app

EXPOSE 8080
EXPOSE 5050/udp

# Switch to the non-privileged user
USER reminisce:reminisce

HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["reminisce"]