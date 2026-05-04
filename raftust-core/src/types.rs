pub type NodeId = u64;
pub type Term = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub term: Term,
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Follower,
    Candidate,
    Leader,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestVote {
    pub term: Term,
    pub candidate_id: NodeId,
    pub last_log_index: usize,
    pub last_log_term: Term,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestVoteResponse {
    pub term: Term,
    pub vote_granted: bool,
    pub from: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendEntries {
    pub term: Term,
    pub leader_id: NodeId,
    pub prev_log_index: usize,
    pub prev_log_term: Term,
    pub entries: Vec<LogEntry>,
    pub leader_commit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendEntriesResponse {
    pub term: Term,
    pub success: bool,
    pub from: NodeId,
    pub match_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSnapshot {
    pub term: Term,
    pub leader_id: NodeId,
    pub last_included_index: usize,
    pub last_included_term: Term,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSnapshotResponse {
    pub term: Term,
    pub from: NodeId,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundMessage {
    RequestVote { to: NodeId, message: RequestVote },
    AppendEntries { to: NodeId, message: AppendEntries },
}
