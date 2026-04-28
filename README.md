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
