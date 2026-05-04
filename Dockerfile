FROM rust:1.88-bookworm AS builder
WORKDIR /app

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p raftust-examples --bin https_file \
    && cp target/release/https_file /https_file

FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
	&& apt-get install -y --no-install-recommends netcat-openbsd ca-certificates curl \
	&& rm -rf /var/lib/apt/lists/*

COPY --from=builder /https_file /usr/local/bin/https_file

ENTRYPOINT ["/usr/local/bin/https_file"]
CMD ["--help"]
