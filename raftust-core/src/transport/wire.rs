use crate::{AppendEntries, LogEntry, NodeId, OutboundRpc, RequestVote};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireLogEntry {
    pub term: u64,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WirePayload {
    RequestVote {
        term: u64,
        candidate_id: NodeId,
        last_log_index: usize,
        last_log_term: u64,
    },
    RequestVoteResponse {
        term: u64,
        vote_granted: bool,
    },
    AppendEntries {
        term: u64,
        leader_id: NodeId,
        prev_log_index: usize,
        prev_log_term: u64,
        entries: Vec<WireLogEntry>,
        leader_commit: usize,
    },
    AppendEntriesResponse {
        term: u64,
        success: bool,
        match_index: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireMessage {
    pub from: NodeId,
    pub to: NodeId,
    pub payload: WirePayload,
}

pub fn outbound_rpc_to_wire(from: NodeId, rpc: OutboundRpc) -> WireMessage {
    match rpc {
        OutboundRpc::RequestVote { to, rpc } => WireMessage {
            from,
            to,
            payload: WirePayload::RequestVote {
                term: rpc.term,
                candidate_id: rpc.candidate_id,
                last_log_index: rpc.last_log_index,
                last_log_term: rpc.last_log_term,
            },
        },
        OutboundRpc::AppendEntries { to, rpc } => WireMessage {
            from,
            to,
            payload: WirePayload::AppendEntries {
                term: rpc.term,
                leader_id: rpc.leader_id,
                prev_log_index: rpc.prev_log_index,
                prev_log_term: rpc.prev_log_term,
                entries: rpc.entries.into_iter().map(to_wire_log_entry).collect(),
                leader_commit: rpc.leader_commit,
            },
        },
    }
}

pub fn from_wire_request_vote(
    term: u64,
    candidate_id: NodeId,
    last_log_index: usize,
    last_log_term: u64,
) -> RequestVote {
    RequestVote {
        term,
        candidate_id,
        last_log_index,
        last_log_term,
    }
}

pub fn from_wire_append_entries(
    term: u64,
    leader_id: NodeId,
    prev_log_index: usize,
    prev_log_term: u64,
    entries: Vec<WireLogEntry>,
    leader_commit: usize,
) -> AppendEntries {
    AppendEntries {
        term,
        leader_id,
        prev_log_index,
        prev_log_term,
        entries: entries.into_iter().map(from_wire_log_entry).collect(),
        leader_commit,
    }
}

fn to_wire_log_entry(entry: LogEntry) -> WireLogEntry {
    WireLogEntry {
        term: entry.term,
        command: entry.command,
    }
}

fn from_wire_log_entry(entry: WireLogEntry) -> LogEntry {
    LogEntry {
        term: entry.term,
        command: entry.command,
    }
}
