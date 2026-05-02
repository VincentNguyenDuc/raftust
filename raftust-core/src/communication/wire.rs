//! Shared wire types and message conversion used by all transport implementations.

use serde::{Deserialize, Serialize};

use super::{InboundMessage, RaftMessage};
use crate::{
    AppendEntries, AppendEntriesResponse, InstallSnapshot, InstallSnapshotResponse, LogEntry,
    NodeId, RequestVote, RequestVoteResponse,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireLogEntry {
    pub term: u64,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(super) enum WirePayload {
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
    InstallSnapshot {
        term: u64,
        leader_id: NodeId,
        last_included_index: usize,
        last_included_term: u64,
        data: Vec<u8>,
    },
    InstallSnapshotResponse {
        term: u64,
        success: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireMessage {
    pub from: NodeId,
    pub to: NodeId,
    pub payload: WirePayload,
}

pub(super) fn message_to_wire(from: NodeId, to: NodeId, message: RaftMessage) -> WireMessage {
    match message {
        RaftMessage::RequestVote(m) => WireMessage {
            from,
            to,
            payload: WirePayload::RequestVote {
                term: m.term,
                candidate_id: m.candidate_id,
                last_log_index: m.last_log_index,
                last_log_term: m.last_log_term,
            },
        },
        RaftMessage::RequestVoteResponse(m) => WireMessage {
            from,
            to,
            payload: WirePayload::RequestVoteResponse {
                term: m.term,
                vote_granted: m.vote_granted,
            },
        },
        RaftMessage::AppendEntries(m) => WireMessage {
            from,
            to,
            payload: WirePayload::AppendEntries {
                term: m.term,
                leader_id: m.leader_id,
                prev_log_index: m.prev_log_index,
                prev_log_term: m.prev_log_term,
                entries: m
                    .entries
                    .into_iter()
                    .map(|e| WireLogEntry {
                        term: e.term,
                        command: e.command,
                    })
                    .collect(),
                leader_commit: m.leader_commit,
            },
        },
        RaftMessage::AppendEntriesResponse(m) => WireMessage {
            from,
            to,
            payload: WirePayload::AppendEntriesResponse {
                term: m.term,
                success: m.success,
                match_index: m.match_index,
            },
        },
        RaftMessage::InstallSnapshot(m) => WireMessage {
            from,
            to,
            payload: WirePayload::InstallSnapshot {
                term: m.term,
                leader_id: m.leader_id,
                last_included_index: m.last_included_index,
                last_included_term: m.last_included_term,
                data: m.data,
            },
        },
        RaftMessage::InstallSnapshotResponse(m) => WireMessage {
            from,
            to,
            payload: WirePayload::InstallSnapshotResponse {
                term: m.term,
                success: m.success,
            },
        },
    }
}

pub(super) fn wire_to_message(msg: WireMessage) -> InboundMessage {
    let message = match msg.payload {
        WirePayload::RequestVote {
            term,
            candidate_id,
            last_log_index,
            last_log_term,
        } => RaftMessage::RequestVote(RequestVote {
            term,
            candidate_id,
            last_log_index,
            last_log_term,
        }),
        WirePayload::RequestVoteResponse { term, vote_granted } => {
            RaftMessage::RequestVoteResponse(RequestVoteResponse {
                term,
                vote_granted,
                from: msg.from,
            })
        }
        WirePayload::AppendEntries {
            term,
            leader_id,
            prev_log_index,
            prev_log_term,
            entries,
            leader_commit,
        } => RaftMessage::AppendEntries(AppendEntries {
            term,
            leader_id,
            prev_log_index,
            prev_log_term,
            entries: entries
                .into_iter()
                .map(|e| LogEntry {
                    term: e.term,
                    command: e.command,
                })
                .collect(),
            leader_commit,
        }),
        WirePayload::AppendEntriesResponse {
            term,
            success,
            match_index,
        } => RaftMessage::AppendEntriesResponse(AppendEntriesResponse {
            term,
            success,
            from: msg.from,
            match_index,
        }),
        WirePayload::InstallSnapshot {
            term,
            leader_id,
            last_included_index,
            last_included_term,
            data,
        } => RaftMessage::InstallSnapshot(InstallSnapshot {
            term,
            leader_id,
            last_included_index,
            last_included_term,
            data,
        }),
        WirePayload::InstallSnapshotResponse { term, success } => {
            RaftMessage::InstallSnapshotResponse(InstallSnapshotResponse {
                term,
                from: msg.from,
                success,
            })
        }
    };

    InboundMessage {
        from: msg.from,
        message,
    }
}
