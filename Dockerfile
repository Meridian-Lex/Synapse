# Stage 1: build
FROM rust:1.88-slim AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .
RUN cargo build --release -p synapse-broker

# Stage 2: runtime
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y ca-certificates libssl3 postgresql-client gettext-base \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/synapse-broker /usr/local/bin/synapse-broker
COPY --from=builder /build/webui/ /app/webui/
COPY migrations/ /app/migrations/
COPY entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh
EXPOSE 7777 7778
ENTRYPOINT ["/app/entrypoint.sh"]
