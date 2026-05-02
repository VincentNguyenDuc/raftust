FROM rust:1.88-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY raftust-core/Cargo.toml raftust-core/Cargo.toml
COPY raftust-example/Cargo.toml raftust-example/Cargo.toml
COPY raftust-core/src raftust-core/src
COPY raftust-example/src raftust-example/src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p raftust-example \
    && cp target/release/raftust-example /raftust-example

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
	&& apt-get install -y --no-install-recommends netcat-openbsd ca-certificates curl \
	&& rm -rf /var/lib/apt/lists/*

COPY --from=builder /raftust-example /usr/local/bin/raftust-example

ENTRYPOINT ["/usr/local/bin/raftust-example"]
