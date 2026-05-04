# Raftust

Raftust is a lightweight, modular implementation of the Raft consensus protocol in Rust.

## Overview

The project is designed around a simple idea: keep consensus logic focused and independent, then connect it to the outside world through interchangeable adapters.

In practice, this means Raftust emphasizes:

- clear separation of responsibilities
- composable building blocks
- easy testing in isolated environments
- flexibility in transport and persistence choices

## Philosophy

Raftust treats consensus as a reusable engine rather than a fixed application stack. Network behavior and storage behavior are intentionally pluggable so teams can choose the operational model that fits their environment.

This approach makes it easier to:

- experiment with different deployment patterns
- swap infrastructure decisions without rewriting core protocol behavior
- reason about correctness separately from integration concerns

## Project Direction

The repository is organized to keep the protocol core minimal and keep optional capabilities modular. As the project evolves, this structure supports adding new integrations while preserving a stable consensus foundation.

## Examples

The `raftust-examples` package contains runnable binaries that show different stack combinations.

- `grpc_in_memory`: gRPC transport with in-memory storage
- `https_file`: HTTPS transport with file-backed storage
- `grpc_counter`: gRPC transport with in-memory storage and counter state machine

Run examples with:

```bash
cargo run -p raftust-examples --bin grpc_in_memory -- --id <id> --addr <host:port> --peer <id=host:port> [--peer ...] --election-timeout-min 20 --election-timeout-max 40 --heartbeat-interval 4
cargo run -p raftust-examples --bin https_file -- --id <id> --addr <host:port> --peer <id=host:port> [--peer ...] --election-timeout-min 20 --election-timeout-max 40 --heartbeat-interval 4
cargo run -p raftust-examples --bin grpc_counter -- --id <id> --addr <host:port> --peer <id=host:port> [--peer ...] --election-timeout-min 20 --election-timeout-max 40 --heartbeat-interval 4
```

Timing options:

- `--election-timeout-min <ticks>`: minimum election timeout
- `--election-timeout-max <ticks>`: maximum election timeout
- `--heartbeat-interval <ticks>`: heartbeat interval, must be less than election-timeout-min

## Docker

Build and run the HTTPS example directly:

```bash
docker build -t raftust .
docker run --rm -it raftust --help
```

Run the 5-node HTTPS cluster with persistent per-node storage:

```bash
docker compose up --build
```

Common runtime commands after startup:

- `status`
- `election`
- `propose <value>`
- `quit`
