# Stage 1: build
FROM rust:1.88-slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build

# Copy workspace manifests and fetch dependencies early for better caching
COPY Cargo.toml Cargo.lock ./
COPY crates/synapse-proto/Cargo.toml crates/synapse-proto/
COPY crates/synapse-broker/Cargo.toml crates/synapse-broker/
COPY crates/synapse-cli/Cargo.toml crates/synapse-cli/
RUN mkdir -p crates/synapse-proto/src && echo "" > crates/synapse-proto/src/lib.rs \
 && mkdir -p crates/synapse-broker/src && echo "fn main(){}" > crates/synapse-broker/src/main.rs \
 && mkdir -p crates/synapse-cli/src && echo "fn main(){}" > crates/synapse-cli/src/main.rs
RUN cargo fetch

# Copy full source and build
COPY . .
RUN cargo build --release -p synapse-broker

# Stage 2: runtime
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 postgresql-client gettext-base \
 && rm -rf /var/lib/apt/lists/*

# Create non-root user for runtime
RUN addgroup --system appuser && adduser --system --ingroup appuser appuser

WORKDIR /app
COPY --from=builder /build/target/release/synapse-broker /usr/local/bin/synapse-broker
COPY --from=builder /build/webui/ /app/webui/
COPY migrations/ /app/migrations/
COPY entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh
RUN chown -R appuser:appuser /app

EXPOSE 7777 7778
HEALTHCHECK --interval=30s --timeout=10s --retries=3 --start-period=15s \
    CMD ["bash", "-c", "echo > /dev/tcp/127.0.0.1/7777"]

USER appuser
ENTRYPOINT ["/app/entrypoint.sh"]
