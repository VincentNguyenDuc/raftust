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
