FROM rust:1.87-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY raftust-core/Cargo.toml raftust-core/Cargo.toml
COPY raftust-example/Cargo.toml raftust-example/Cargo.toml
COPY raftust-core/src raftust-core/src
COPY raftust-example/src raftust-example/src

RUN cargo build --release -p raftust-example

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
	&& apt-get install -y --no-install-recommends netcat-openbsd ca-certificates \
	&& rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/raftust-example /usr/local/bin/raftust-example

ENTRYPOINT ["/usr/local/bin/raftust-example"]
