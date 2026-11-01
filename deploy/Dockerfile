# Build stage
FROM rust:1.80-bullseye as builder
WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY crates/pctunnel-proto/Cargo.toml crates/pctunnel-proto/
COPY crates/pctunnel-relay/Cargo.toml crates/pctunnel-relay/
COPY crates/pctunnel-core/Cargo.toml crates/pctunnel-core/
COPY crates/pctunnel-app/Cargo.toml crates/pctunnel-app/
COPY crates/pctunnel-mcp/Cargo.toml crates/pctunnel-mcp/

# Build dependencies only for caching
RUN mkdir -p crates/pctunnel-proto/src crates/pctunnel-relay/src crates/pctunnel-core/src crates/pctunnel-app/src crates/pctunnel-mcp/src && \
    echo "pub fn dummy() {}" > crates/pctunnel-proto/src/lib.rs && \
    echo "fn main() {}" > crates/pctunnel-relay/src/main.rs && \
    echo "pub fn dummy() {}" > crates/pctunnel-core/src/lib.rs && \
    echo "fn main() {}" > crates/pctunnel-app/src/main.rs && \
    echo "fn main() {}" > crates/pctunnel-mcp/src/main.rs && \
    cargo build --release -p pctunnel-relay

# Copy real sources
COPY crates/ crates/

# Touch main.rs to invalidate build
RUN touch crates/pctunnel-relay/src/main.rs && \
    cargo build --release -p pctunnel-relay

# Runtime stage: slim and secure
FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata && rm -rf /var/lib/apt/lists/*
RUN useradd -u 10001 -m -s /bin/false pctunnel

USER pctunnel
WORKDIR /home/pctunnel

COPY --from=builder /app/target/release/pctunnel-relay /usr/local/bin/pctunnel-relay

ENV PCTUNNEL_BIND=0.0.0.0:8080
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/pctunnel-relay"]
