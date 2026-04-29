# Raftust
An implementation of Raft

## Current Scope

The `raftust-core` crate currently includes:

- Raft roles (`Follower`, `Candidate`, `Leader`)
- Raft RPC types (`RequestVote`, `AppendEntries` and responses)
- Leader election state transitions
- Basic append-entries conflict handling
- Unit tests for election and log consistency basics

## Run

```bash
cargo test
cargo run -p raftust-core
```

## Phase 1 Local Multi-Process Cluster

Start 3 terminals and run one node per terminal:

```bash
cargo run -p raftust-core -- --id 1 --addr 127.0.0.1:5001 --peer 2=127.0.0.1:5002 --peer 3=127.0.0.1:5003
cargo run -p raftust-core -- --id 2 --addr 127.0.0.1:5002 --peer 1=127.0.0.1:5001 --peer 3=127.0.0.1:5003
cargo run -p raftust-core -- --id 3 --addr 127.0.0.1:5003 --peer 1=127.0.0.1:5001 --peer 2=127.0.0.1:5002
```

Each node accepts commands on stdin:

- `status`
- `election`
- `propose <value>`
- `quit`
