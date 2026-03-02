.PHONY: all broker cli-linux cli-windows test clean mcp-build

all: broker cli-linux cli-windows mcp-build

broker:
	cargo build --release -p synapse-broker --target x86_64-unknown-linux-musl

cli-linux:
	cargo build --release -p synapse-cli --target x86_64-unknown-linux-musl

cli-windows:
	cargo build --release -p synapse-cli --target x86_64-pc-windows-gnu

test:
	cargo test --workspace

clean:
	cargo clean

mcp-build:
	cd mcp && npm install && npm run build
