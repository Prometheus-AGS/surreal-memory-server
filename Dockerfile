# syntax=docker/dockerfile:1.4
# ── Stage 1: Build ─────────────────────────────────────────────────────────────
FROM rust:1.93-slim AS builder

RUN apt-get update && apt-get install -y \
    clang \
    libclang-dev \
    pkg-config \
    build-essential \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies separately from source (layer cache optimization)
COPY Cargo.toml Cargo.lock ./
COPY crates/surreal-memory/Cargo.toml crates/surreal-memory/
RUN mkdir -p src crates/surreal-memory/src \
    && echo "fn main() {}" > src/main.rs \
    && echo "" > crates/surreal-memory/src/lib.rs \
    && cargo build --release --bin surreal-memory-server 2>/dev/null || true

# Now copy and build the real source
COPY . .
RUN touch src/main.rs crates/surreal-memory/src/lib.rs && cargo build --release --bin surreal-memory-server

# ── Stage 2: Runtime ───────────────────────────────────────────────────────────
FROM debian:trixie-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 libstdc++6 \
    && rm -rf /var/lib/apt/lists/*


COPY --from=builder /app/target/release/surreal-memory-server /usr/local/bin/surreal-memory-server

# Non-root user for security
RUN useradd -m -u 1001 smserver
USER smserver

WORKDIR /data

ENV RUST_LOG=info \
    API_PORT=3001 \
    SURREAL_MODE=embedded \
    SURREAL_EMBEDDED_PATH=/data/memory.db \
    SURREAL_NAMESPACE=memory \
    SURREAL_DATABASE=main

EXPOSE 3001

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:${API_PORT}/health || exit 1

CMD ["surreal-memory-server"]
