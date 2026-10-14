# Stage 1: Protocol Buffers Code Generation via Buf
FROM bufbuild/buf:1.43.0 AS buf-stage

WORKDIR /buf

COPY buf.yaml buf.gen.yaml* buf.lock* ./
COPY proto ./proto

RUN buf generate --template buf.gen.yaml

# Stage 2: Rust Dependencies & Binary Builder
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# Install binutils for stripping binary symbols
RUN apt-get update && apt-get install -y --no-install-recommends \
    binutils \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency manifests
COPY Cargo.toml Cargo.lock ./

# Copy generated code from buf-stage and handwritten mod.rs
COPY --from=buf-stage /buf/src/gen ./src/gen
COPY src/gen/mod.rs ./src/gen/mod.rs

# Pre-compile dependencies using skeleton sources for optimal layer caching
RUN mkdir -p src/bin benches && \
    echo "pub mod gen;" > src/lib.rs && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/benchmark.rs && \
    echo "fn main() {}" > benches/orderbook.rs && \
    echo "fn main() {}" > benches/wal.rs && \
    echo "fn main() {}" > benches/engine.rs && \
    cargo build --release --bin matcher

# Copy actual project source code
COPY src ./src
COPY config.example.toml ./

# Touch files to invalidate dummy artifact and compile optimized release binary
RUN touch src/main.rs src/lib.rs && \
    cargo build --release --bin matcher && \
    strip /app/target/release/matcher

# Stage 3: Minimal Unprivileged Runtime Container
FROM debian:bookworm-slim AS runtime

# Install certificates and grpc_health_probe for liveness/readiness probes
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && curl -sSL -o /usr/local/bin/grpc-health-probe https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/v0.4.37/grpc_health_probe-linux-amd64 \
    && chmod +x /usr/local/bin/grpc-health-probe \
    && apt-get purge -y curl \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

# Create unprivileged system group and user
RUN groupadd -g 10001 matcher && \
    useradd -u 10001 -g matcher -d /app -s /usr/sbin/nologin matcher && \
    mkdir -p /app/data/wal && \
    chown -R matcher:matcher /app

WORKDIR /app

# Copy binary and default configuration
COPY --from=builder /app/target/release/matcher /usr/local/bin/matcher
COPY --from=builder --chown=matcher:matcher /app/config.example.toml /app/config.toml

# Switch to unprivileged user
USER matcher

# Expose default gRPC port
EXPOSE 50051

# Persist WAL and snapshot directory
VOLUME ["/app/data"]

# Standard gRPC health check probe
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/grpc-health-probe", "-addr=:50051"]

ENTRYPOINT ["matcher"]
CMD ["--config", "/app/config.toml"]