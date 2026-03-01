# Multi-stage build: compile with musl for static binary, deploy from scratch
# Stage 1: build (requires musl-tools: apt install musl-tools)
# cargo build --release -p synapse-broker --target x86_64-unknown-linux-musl
FROM scratch
COPY target/x86_64-unknown-linux-musl/release/synapse-broker /synapse-broker
COPY webui/ /webui/
EXPOSE 7777 7778
ENTRYPOINT ["/synapse-broker"]
