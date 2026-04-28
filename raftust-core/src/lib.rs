use std::collections::HashSet;

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
pub enum OutboundRpc {
    RequestVote { to: NodeId, rpc: RequestVote },
    AppendEntries { to: NodeId, rpc: AppendEntries },
}

#[derive(Debug)]
pub struct RaftNode {
    pub id: NodeId,
    pub peers: Vec<NodeId>,
    pub current_term: Term,
    pub voted_for: Option<NodeId>,
    pub log: Vec<LogEntry>,
    pub commit_index: usize,
    pub last_applied: usize,
    pub role: Role,
    pub leader_id: Option<NodeId>,
    votes_received: HashSet<NodeId>,
    election_elapsed: u64,
    election_timeout: u64,
    heartbeat_elapsed: u64,
    heartbeat_interval: u64,
}

impl RaftNode {
    pub fn new(
        id: NodeId,
        peers: Vec<NodeId>,
        election_timeout: u64,
        heartbeat_interval: u64,
    ) -> Self {
        Self {
            id,
            peers,
            current_term: 0,
            voted_for: None,
            log: Vec::new(),
            commit_index: 0,
            last_applied: 0,
            role: Role::Follower,
            leader_id: None,
            votes_received: HashSet::new(),
            election_elapsed: 0,
            election_timeout,
            heartbeat_elapsed: 0,
            heartbeat_interval,
        }
    }

    pub fn majority(&self) -> usize {
        // Cluster size is self + all peers.
        (self.peers.len() + 1) / 2 + 1
    }

    pub fn tick(&mut self) -> Vec<OutboundRpc> {
        match self.role {
            Role::Leader => {
                self.heartbeat_elapsed += 1;
                if self.heartbeat_elapsed >= self.heartbeat_interval {
                    self.heartbeat_elapsed = 0;
                    self.build_heartbeat_rpcs()
                } else {
                    Vec::new()
                }
            }
            Role::Follower | Role::Candidate => {
                self.election_elapsed += 1;
                if self.election_elapsed >= self.election_timeout {
                    self.start_election()
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub fn start_election(&mut self) -> Vec<OutboundRpc> {
        self.role = Role::Candidate;
        self.current_term += 1;
        self.voted_for = Some(self.id);
        self.leader_id = None;
        self.election_elapsed = 0;
        self.votes_received.clear();
        self.votes_received.insert(self.id);

        let (last_log_index, last_log_term) = self.last_log_info();

        self.peers
            .iter()
            .copied()
            .map(|peer| OutboundRpc::RequestVote {
                to: peer,
                rpc: RequestVote {
                    term: self.current_term,
                    candidate_id: self.id,
                    last_log_index,
                    last_log_term,
                },
            })
            .collect()
    }

    pub fn handle_request_vote(&mut self, req: RequestVote) -> RequestVoteResponse {
        if req.term < self.current_term {
            return RequestVoteResponse {
                term: self.current_term,
                vote_granted: false,
                from: self.id,
            };
        }

        if req.term > self.current_term {
            self.become_follower(req.term, None);
        }

        let (my_last_index, my_last_term) = self.last_log_info();
        let candidate_log_is_up_to_date = req.last_log_term > my_last_term
            || (req.last_log_term == my_last_term && req.last_log_index >= my_last_index);

        let can_vote = self.voted_for.is_none() || self.voted_for == Some(req.candidate_id);
        let grant = can_vote && candidate_log_is_up_to_date;

        if grant {
            self.voted_for = Some(req.candidate_id);
            self.election_elapsed = 0;
        }

        RequestVoteResponse {
            term: self.current_term,
            vote_granted: grant,
            from: self.id,
        }
    }

    pub fn handle_request_vote_response(&mut self, resp: RequestVoteResponse) -> bool {
        if resp.term > self.current_term {
            self.become_follower(resp.term, None);
            return false;
        }

        if self.role != Role::Candidate || resp.term != self.current_term {
            return false;
        }

        if resp.vote_granted {
            self.votes_received.insert(resp.from);
        }

        if self.votes_received.len() >= self.majority() {
            self.become_leader();
            return true;
        }

        false
    }

    pub fn handle_append_entries(&mut self, req: AppendEntries) -> AppendEntriesResponse {
        if req.term < self.current_term {
            return AppendEntriesResponse {
                term: self.current_term,
                success: false,
                from: self.id,
                match_index: self.log.len(),
            };
        }

        if req.term > self.current_term || self.role != Role::Follower {
            self.become_follower(req.term, Some(req.leader_id));
        }
        self.leader_id = Some(req.leader_id);
        self.election_elapsed = 0;

        // Log indices in the protocol are 1-based. Internal vec indices are 0-based.
        if req.prev_log_index > self.log.len() {
            return AppendEntriesResponse {
                term: self.current_term,
                success: false,
                from: self.id,
                match_index: self.log.len(),
            };
        }

        if req.prev_log_index > 0 {
            let prev_idx = req.prev_log_index - 1;
            if self.log[prev_idx].term != req.prev_log_term {
                return AppendEntriesResponse {
                    term: self.current_term,
                    success: false,
                    from: self.id,
                    match_index: prev_idx,
                };
            }
        }

        let mut insert_at = req.prev_log_index;
        for entry in req.entries {
            if insert_at < self.log.len() {
                if self.log[insert_at].term != entry.term {
                    self.log.truncate(insert_at);
                    self.log.push(entry);
                }
            } else {
                self.log.push(entry);
            }
            insert_at += 1;
        }

        if req.leader_commit > self.commit_index {
            self.commit_index = req.leader_commit.min(self.log.len());
        }

        AppendEntriesResponse {
            term: self.current_term,
            success: true,
            from: self.id,
            match_index: self.log.len(),
        }
    }

    pub fn propose_command(&mut self, command: impl Into<String>) -> Option<Vec<OutboundRpc>> {
        if self.role != Role::Leader {
            return None;
        }

        self.log.push(LogEntry {
            term: self.current_term,
            command: command.into(),
        });

        Some(self.build_heartbeat_rpcs())
    }

    fn become_follower(&mut self, term: Term, leader: Option<NodeId>) {
        self.role = Role::Follower;
        self.current_term = term;
        self.voted_for = None;
        self.votes_received.clear();
        self.election_elapsed = 0;
        self.heartbeat_elapsed = 0;
        self.leader_id = leader;
    }

    fn become_leader(&mut self) {
        self.role = Role::Leader;
        self.leader_id = Some(self.id);
        self.heartbeat_elapsed = 0;
    }

    fn build_heartbeat_rpcs(&self) -> Vec<OutboundRpc> {
        let (last_log_index, last_log_term) = self.last_log_info();
        self.peers
            .iter()
            .copied()
            .map(|peer| OutboundRpc::AppendEntries {
                to: peer,
                rpc: AppendEntries {
                    term: self.current_term,
                    leader_id: self.id,
                    prev_log_index: last_log_index,
                    prev_log_term: last_log_term,
                    entries: Vec::new(),
                    leader_commit: self.commit_index,
                },
            })
            .collect()
    }

    fn last_log_info(&self) -> (usize, Term) {
        if let Some(last) = self.log.last() {
            (self.log.len(), last.term)
        } else {
            (0, 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: NodeId) -> RaftNode {
        let peers = vec![1, 2, 3]
            .into_iter()
            .filter(|peer| *peer != id)
            .collect::<Vec<_>>();
        RaftNode::new(id, peers, 3, 1)
    }

    #[test]
    fn start_election_votes_for_self_and_sends_requests() {
        let mut n = node(1);
        let rpcs = n.start_election();

        assert_eq!(n.role, Role::Candidate);
        assert_eq!(n.current_term, 1);
        assert_eq!(n.voted_for, Some(1));
        assert_eq!(rpcs.len(), 2);
    }

    #[test]
    fn candidate_becomes_leader_after_majority_votes() {
        let mut n = node(1);
        n.start_election();

        let elected = n.handle_request_vote_response(RequestVoteResponse {
            term: 1,
            vote_granted: true,
            from: 2,
        });

        assert!(elected);
        assert_eq!(n.role, Role::Leader);
        assert_eq!(n.leader_id, Some(1));
    }

    #[test]
    fn request_vote_rejects_stale_log() {
        let mut n = node(2);
        n.log.push(LogEntry {
            term: 3,
            command: "x=1".to_string(),
        });
        n.current_term = 3;

        let resp = n.handle_request_vote(RequestVote {
            term: 3,
            candidate_id: 1,
            last_log_index: 0,
            last_log_term: 0,
        });

        assert!(!resp.vote_granted);
    }

    #[test]
    fn request_vote_grants_if_log_is_up_to_date() {
        let mut n = node(2);
        n.current_term = 2;

        let resp = n.handle_request_vote(RequestVote {
            term: 2,
            candidate_id: 1,
            last_log_index: 0,
            last_log_term: 0,
        });

        assert!(resp.vote_granted);
        assert_eq!(n.voted_for, Some(1));
    }

    #[test]
    fn append_entries_rewrites_conflicting_suffix() {
        let mut n = node(2);
        n.current_term = 2;
        n.log = vec![
            LogEntry {
                term: 1,
                command: "a".to_string(),
            },
            LogEntry {
                term: 2,
                command: "b".to_string(),
            },
        ];

        let resp = n.handle_append_entries(AppendEntries {
            term: 3,
            leader_id: 1,
            prev_log_index: 1,
            prev_log_term: 1,
            entries: vec![LogEntry {
                term: 3,
                command: "c".to_string(),
            }],
            leader_commit: 2,
        });

        assert!(resp.success);
        assert_eq!(n.log.len(), 2);
        assert_eq!(n.log[1].term, 3);
        assert_eq!(n.log[1].command, "c");
        assert_eq!(n.commit_index, 2);
    }
}
