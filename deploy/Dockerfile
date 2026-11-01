# Build stage
FROM rust:1.80-bullseye as builder
WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates/omnilink-proto/Cargo.toml crates/omnilink-proto/
COPY crates/omnilink-relay/Cargo.toml crates/omnilink-relay/
COPY crates/omnilink-core/Cargo.toml crates/omnilink-core/
COPY crates/omnilink-app/Cargo.toml crates/omnilink-app/
COPY crates/omnilink-mcp/Cargo.toml crates/omnilink-mcp/

# Build dependencies only for caching
RUN mkdir -p crates/omnilink-proto/src crates/omnilink-relay/src crates/omnilink-core/src crates/omnilink-app/src crates/omnilink-mcp/src && \
    echo "pub fn dummy() {}" > crates/omnilink-proto/src/lib.rs && \
    echo "fn main() {}" > crates/omnilink-relay/src/main.rs && \
    echo "pub fn dummy() {}" > crates/omnilink-core/src/lib.rs && \
    echo "fn main() {}" > crates/omnilink-app/src/main.rs && \
    echo "fn main() {}" > crates/omnilink-mcp/src/main.rs && \
    cargo build --release -p omnilink-relay

# Copy real sources
COPY crates/ crates/

# Touch main.rs to invalidate build
RUN touch crates/omnilink-relay/src/main.rs && \
    cargo build --release -p omnilink-relay

# Runtime stage: slim and secure
FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata && rm -rf /var/lib/apt/lists/*
RUN useradd -u 10001 -m -s /bin/false omnilink

USER omnilink
WORKDIR /home/omnilink

COPY --from=builder /app/target/release/omnilink-relay /usr/local/bin/omnilink-relay

ENV OMNILINK_BIND=0.0.0.0:8080
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/omnilink-relay"]
