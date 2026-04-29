use std::collections::{HashMap, HashSet};

use crate::state_machine::apply_command;
use crate::types::{
    AppendEntries, AppendEntriesResponse, LogEntry, NodeId, OutboundRpc, RequestVote,
    RequestVoteResponse, Role, Term,
};

#[derive(Debug)]
pub struct RaftNode {
    pub id: NodeId,
    pub peers: Vec<NodeId>,
    pub current_term: Term,
    pub voted_for: Option<NodeId>,
    pub log: Vec<LogEntry>,
    pub commit_index: usize,
    pub last_applied: usize,
    pub state_machine: HashMap<String, String>,
    pub role: Role,
    pub leader_id: Option<NodeId>,
    leader_next_index: HashMap<NodeId, usize>,
    leader_match_index: HashMap<NodeId, usize>,
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
            state_machine: HashMap::new(),
            role: Role::Follower,
            leader_id: None,
            leader_next_index: HashMap::new(),
            leader_match_index: HashMap::new(),
            votes_received: HashSet::new(),
            election_elapsed: 0,
            election_timeout,
            heartbeat_elapsed: 0,
            heartbeat_interval,
        }
    }

    pub fn majority(&self) -> usize {
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
            self.commit_to(req.leader_commit.min(self.log.len()));
        }

        AppendEntriesResponse {
            term: self.current_term,
            success: true,
            from: self.id,
            match_index: self.log.len(),
        }
    }

    pub fn handle_append_entries_response(
        &mut self,
        resp: AppendEntriesResponse,
    ) -> Vec<OutboundRpc> {
        if resp.term > self.current_term {
            self.become_follower(resp.term, None);
            return Vec::new();
        }

        if self.role != Role::Leader || resp.term != self.current_term {
            return Vec::new();
        }

        if resp.success {
            self.leader_match_index.insert(resp.from, resp.match_index);
            self.leader_next_index
                .insert(resp.from, resp.match_index + 1);
            self.try_advance_commit_index();
            return Vec::new();
        }

        let current_next = self
            .leader_next_index
            .get(&resp.from)
            .copied()
            .unwrap_or(self.log.len() + 1);
        let new_next = current_next.saturating_sub(1).max(1);
        self.leader_next_index.insert(resp.from, new_next);

        match self.build_append_entries_for_peer(resp.from) {
            Some(rpc) => vec![OutboundRpc::AppendEntries { to: resp.from, rpc }],
            None => Vec::new(),
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
        self.leader_match_index.insert(self.id, self.log.len());

        if self.majority() == 1 {
            self.commit_to(self.log.len());
        }

        Some(self.build_heartbeat_rpcs())
    }

    pub fn commit_to(&mut self, new_commit_index: usize) {
        self.commit_index = new_commit_index.min(self.log.len());
        self.apply_committed_entries();
    }

    pub fn state_machine_get(&self, key: &str) -> Option<&str> {
        self.state_machine.get(key).map(String::as_str)
    }

    fn become_follower(&mut self, term: Term, leader: Option<NodeId>) {
        self.role = Role::Follower;
        self.current_term = term;
        self.voted_for = None;
        self.votes_received.clear();
        self.leader_next_index.clear();
        self.leader_match_index.clear();
        self.election_elapsed = 0;
        self.heartbeat_elapsed = 0;
        self.leader_id = leader;
    }

    fn become_leader(&mut self) {
        self.role = Role::Leader;
        self.leader_id = Some(self.id);
        self.heartbeat_elapsed = 0;
        self.leader_next_index.clear();
        self.leader_match_index.clear();
        self.leader_match_index.insert(self.id, self.log.len());

        let next = self.log.len() + 1;
        for peer in &self.peers {
            self.leader_next_index.insert(*peer, next);
            self.leader_match_index.insert(*peer, 0);
        }
    }

    fn build_heartbeat_rpcs(&self) -> Vec<OutboundRpc> {
        self.peers
            .iter()
            .copied()
            .filter_map(|peer| {
                self.build_append_entries_for_peer(peer)
                    .map(|rpc| OutboundRpc::AppendEntries { to: peer, rpc })
            })
            .collect()
    }

    fn build_append_entries_for_peer(&self, peer: NodeId) -> Option<AppendEntries> {
        let next_idx = self.leader_next_index.get(&peer).copied().unwrap_or(1);
        let prev_log_index = next_idx.saturating_sub(1);
        let prev_log_term = if prev_log_index == 0 {
            0
        } else {
            self.log.get(prev_log_index - 1)?.term
        };

        let entries = if next_idx == 0 || next_idx > self.log.len() {
            Vec::new()
        } else {
            self.log[(next_idx - 1)..].to_vec()
        };

        Some(AppendEntries {
            term: self.current_term,
            leader_id: self.id,
            prev_log_index,
            prev_log_term,
            entries,
            leader_commit: self.commit_index,
        })
    }

    fn last_log_info(&self) -> (usize, Term) {
        if let Some(last) = self.log.last() {
            (self.log.len(), last.term)
        } else {
            (0, 0)
        }
    }

    fn apply_committed_entries(&mut self) {
        while self.last_applied < self.commit_index {
            let idx = self.last_applied;
            apply_command(&mut self.state_machine, &self.log[idx].command);
            self.last_applied += 1;
        }
    }

    fn try_advance_commit_index(&mut self) {
        if self.role != Role::Leader {
            return;
        }

        for idx in (self.commit_index + 1..=self.log.len()).rev() {
            if self.log[idx - 1].term != self.current_term {
                continue;
            }

            let replicated = self
                .leader_match_index
                .values()
                .filter(|match_idx| **match_idx >= idx)
                .count();

            if replicated >= self.majority() {
                self.commit_to(idx);
                break;
            }
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

    #[test]
    fn commit_applies_state_machine_commands() {
        let mut n = node(1);
        n.log = vec![
            LogEntry {
                term: 1,
                command: "set color blue".to_string(),
            },
            LogEntry {
                term: 1,
                command: "set size large".to_string(),
            },
            LogEntry {
                term: 1,
                command: "del color".to_string(),
            },
        ];

        n.commit_to(3);

        assert_eq!(n.last_applied, 3);
        assert_eq!(n.state_machine_get("size"), Some("large"));
        assert_eq!(n.state_machine_get("color"), None);
    }

    #[test]
    fn append_entries_with_commit_applies_to_state_machine() {
        let mut n = node(2);
        n.current_term = 4;

        let resp = n.handle_append_entries(AppendEntries {
            term: 4,
            leader_id: 1,
            prev_log_index: 0,
            prev_log_term: 0,
            entries: vec![
                LogEntry {
                    term: 4,
                    command: "set animal cat".to_string(),
                },
                LogEntry {
                    term: 4,
                    command: "set mood happy".to_string(),
                },
            ],
            leader_commit: 2,
        });

        assert!(resp.success);
        assert_eq!(n.commit_index, 2);
        assert_eq!(n.last_applied, 2);
        assert_eq!(n.state_machine_get("animal"), Some("cat"));
        assert_eq!(n.state_machine_get("mood"), Some("happy"));
    }

    #[test]
    fn leader_proposal_sends_entry_to_followers() {
        let mut n = node(1);
        n.start_election();
        assert!(n.handle_request_vote_response(RequestVoteResponse {
            term: 1,
            vote_granted: true,
            from: 2,
        }));

        let outbound = n
            .propose_command("set k v")
            .expect("leader must accept proposal");

        let mut append_count = 0;
        for msg in outbound {
            if let OutboundRpc::AppendEntries { rpc, .. } = msg {
                append_count += 1;
                assert_eq!(rpc.entries.len(), 1);
                assert_eq!(rpc.entries[0].command, "set k v");
            }
        }
        assert_eq!(append_count, 2);
    }

    #[test]
    fn leader_commits_after_majority_ack() {
        let mut n = node(1);
        n.start_election();
        assert!(n.handle_request_vote_response(RequestVoteResponse {
            term: 1,
            vote_granted: true,
            from: 2,
        }));

        n.propose_command("set color red");
        assert_eq!(n.commit_index, 0);

        let retry = n.handle_append_entries_response(AppendEntriesResponse {
            term: 1,
            success: true,
            from: 2,
            match_index: 1,
        });

        assert!(retry.is_empty());
        assert_eq!(n.commit_index, 1);
        assert_eq!(n.last_applied, 1);
        assert_eq!(n.state_machine_get("color"), Some("red"));
    }

    #[test]
    fn leader_retries_with_lower_next_index_after_reject() {
        let mut n = node(1);
        n.start_election();
        assert!(n.handle_request_vote_response(RequestVoteResponse {
            term: 1,
            vote_granted: true,
            from: 2,
        }));
        n.propose_command("set x 1");

        let retry = n.handle_append_entries_response(AppendEntriesResponse {
            term: 1,
            success: false,
            from: 2,
            match_index: 0,
        });

        assert_eq!(retry.len(), 1);
        match &retry[0] {
            OutboundRpc::AppendEntries { to, rpc } => {
                assert_eq!(*to, 2);
                assert_eq!(rpc.prev_log_index, 0);
                assert_eq!(rpc.entries.len(), 1);
            }
            _ => panic!("expected append retry"),
        }
    }
}
