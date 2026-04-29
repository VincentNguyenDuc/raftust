pub mod config;
pub mod transport;

mod core;
pub use core::node::RaftNode;
pub use core::types::{
    AppendEntries, AppendEntriesResponse, LogEntry, NodeId, OutboundRpc, RequestVote,
    RequestVoteResponse, Role, Term,
};
