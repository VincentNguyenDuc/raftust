# Raftust
A modular implementation of Raft in Rust.

## Design

Raftust separates the Raft protocol from the concerns of how nodes communicate and how state is persisted. The protocol logic lives in `raftust-core` and is completely decoupled from transport and storage through two pluggable traits: `RaftCommunication` and `StorageStrategy`. Consumers implement these traits to adapt the library to any transport (TCP, gRPC, in-process channels) or any storage backend (disk, database, in-memory) without touching the protocol code.

The `Runner` ties these pieces together. It owns the tick loop, drives inbound message processing, dispatches outbound messages, and coordinates persistence. It is generic over both pluggable components, so the full cluster behavior can be exercised in tests with in-memory fakes without any network or disk involvement.

`raftust-example` demonstrates the simplest wiring: `LocalNetworkCommunication` (TCP, line-delimited JSON) and `InMemoryStorage`.

## Run

```bash
cargo test
```

Start 3 terminals and run one node per terminal:

```bash
cargo run -p raftust-example -- --id 1 --addr 127.0.0.1:5001 --peer 2=127.0.0.1:5002 --peer 3=127.0.0.1:5003
cargo run -p raftust-example -- --id 2 --addr 127.0.0.1:5002 --peer 1=127.0.0.1:5001 --peer 3=127.0.0.1:5003
cargo run -p raftust-example -- --id 3 --addr 127.0.0.1:5003 --peer 1=127.0.0.1:5001 --peer 2=127.0.0.1:5002
```

Each node accepts commands on stdin:

- `status`
- `election`
- `propose <key> <value>`
- `quit`
