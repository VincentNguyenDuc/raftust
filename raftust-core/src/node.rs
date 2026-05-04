use super::types::{
    AppendEntries, AppendEntriesResponse, LogEntry, NodeId, OutboundMessage, RequestVote,
    RequestVoteResponse, Role, Term,
};
use rand::Rng;
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub struct RaftNode {
    pub id: NodeId,
    pub peers: Vec<NodeId>,
    pub current_term: Term,
    pub voted_for: Option<NodeId>,
    pub log: Vec<LogEntry>,
    pub snapshot_last_included_index: usize,
    pub snapshot_last_included_term: Term,
    pub commit_index: usize,
    pub last_applied: usize,
    pub role: Role,
    pub leader_id: Option<NodeId>,
    leader_next_index: HashMap<NodeId, usize>,
    leader_match_index: HashMap<NodeId, usize>,
    votes_received: HashSet<NodeId>,
    election_elapsed: u64,
    election_timeout_min: u64,
    election_timeout_max: u64,
    current_election_timeout: u64,
    heartbeat_elapsed: u64,
    heartbeat_interval: u64,
}

impl RaftNode {
    pub fn new(
        id: NodeId,
        peers: Vec<NodeId>,
        election_timeout_min: u64,
        election_timeout_max: u64,
        heartbeat_interval: u64,
    ) -> Self {
        assert!(election_timeout_min > 0);
        assert!(election_timeout_max >= election_timeout_min);

        let current_election_timeout =
            Self::sample_election_timeout(election_timeout_min, election_timeout_max);

        Self {
            id,
            peers,
            current_term: 0,
            voted_for: None,
            log: Vec::new(),
            snapshot_last_included_index: 0,
            snapshot_last_included_term: 0,
            commit_index: 0,
            last_applied: 0,
            role: Role::Follower,
            leader_id: None,
            leader_next_index: HashMap::new(),
            leader_match_index: HashMap::new(),
            votes_received: HashSet::new(),
            election_elapsed: 0,
            election_timeout_min,
            election_timeout_max,
            current_election_timeout,
            heartbeat_elapsed: 0,
            heartbeat_interval,
        }
    }

    pub fn majority(&self) -> usize {
        (self.peers.len() + 1) / 2 + 1
    }

    pub fn tick(&mut self) -> Vec<OutboundMessage> {
        match self.role {
            Role::Leader => {
                self.heartbeat_elapsed += 1;
                if self.heartbeat_elapsed >= self.heartbeat_interval {
                    self.heartbeat_elapsed = 0;
                    self.build_heartbeat_messages()
                } else {
                    Vec::new()
                }
            }
            Role::Follower | Role::Candidate => {
                self.election_elapsed += 1;
                if self.election_elapsed >= self.current_election_timeout {
                    self.start_election()
                } else {
                    Vec::new()
                }
            }
        }
    }

    pub fn start_election(&mut self) -> Vec<OutboundMessage> {
        self.role = Role::Candidate;
        self.current_term += 1;
        self.voted_for = Some(self.id);
        self.leader_id = None;
        self.election_elapsed = 0;
        self.current_election_timeout =
            Self::sample_election_timeout(self.election_timeout_min, self.election_timeout_max);
        self.votes_received.clear();
        self.votes_received.insert(self.id);

        let (last_log_index, last_log_term) = self.last_log_info();

        self.peers
            .iter()
            .copied()
            .map(|peer| OutboundMessage::RequestVote {
                to: peer,
                message: RequestVote {
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
            self.current_election_timeout =
                Self::sample_election_timeout(self.election_timeout_min, self.election_timeout_max);
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
                match_index: self.last_log_index(),
            };
        }

        if req.term > self.current_term || self.role != Role::Follower {
            self.become_follower(req.term, Some(req.leader_id));
        }
        self.leader_id = Some(req.leader_id);
        self.election_elapsed = 0;

        if req.prev_log_index < self.snapshot_last_included_index {
            return AppendEntriesResponse {
                term: self.current_term,
                success: false,
                from: self.id,
                match_index: self.snapshot_last_included_index,
            };
        }

        if req.prev_log_index > self.last_log_index() {
            return AppendEntriesResponse {
                term: self.current_term,
                success: false,
                from: self.id,
                match_index: self.last_log_index(),
            };
        }

        if req.prev_log_index > 0 {
            if self.term_at(req.prev_log_index) != Some(req.prev_log_term) {
                return AppendEntriesResponse {
                    term: self.current_term,
                    success: false,
                    from: self.id,
                    match_index: req.prev_log_index.saturating_sub(1),
                };
            }
        }

        let mut insert_at = req.prev_log_index + 1;
        for entry in req.entries {
            if insert_at <= self.snapshot_last_included_index {
                insert_at += 1;
                continue;
            }

            if let Some(offset) = self.offset_for_index(insert_at) {
                if self.log[offset].term != entry.term {
                    self.log.truncate(offset);
                    self.log.push(entry);
                }
            } else {
                self.log.push(entry);
            }
            insert_at += 1;
        }

        if req.leader_commit > self.commit_index {
            self.commit_to(req.leader_commit.min(self.last_log_index()));
        }

        AppendEntriesResponse {
            term: self.current_term,
            success: true,
            from: self.id,
            match_index: self.last_log_index(),
        }
    }

    pub fn handle_append_entries_response(
        &mut self,
        resp: AppendEntriesResponse,
    ) -> Vec<OutboundMessage> {
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
            .unwrap_or(self.last_log_index() + 1);
        let new_next = current_next.saturating_sub(1).max(1);
        self.leader_next_index.insert(resp.from, new_next);

        match self.build_append_entries_for_peer(resp.from) {
            Some(message) => vec![OutboundMessage::AppendEntries {
                to: resp.from,
                message,
            }],
            None => Vec::new(),
        }
    }

    pub fn propose_command(&mut self, command: impl Into<String>) -> Option<Vec<OutboundMessage>> {
        if self.role != Role::Leader {
            return None;
        }

        self.log.push(LogEntry {
            term: self.current_term,
            command: command.into(),
        });
        self.leader_match_index
            .insert(self.id, self.last_log_index());

        if self.majority() == 1 {
            self.commit_to(self.last_log_index());
        }

        Some(self.build_heartbeat_messages())
    }

    pub fn commit_to(&mut self, new_commit_index: usize) {
        let min_commit = self.snapshot_last_included_index;
        let max_commit = self.last_log_index();
        self.commit_index = new_commit_index.clamp(min_commit, max_commit);
    }

    pub fn take_unapplied_entries(&mut self) -> Vec<LogEntry> {
        if self.last_applied < self.snapshot_last_included_index {
            self.last_applied = self.snapshot_last_included_index;
        }

        let end = self.commit_index.min(self.last_log_index());
        if self.last_applied >= end {
            return Vec::new();
        }

        let start_idx = self.last_applied + 1;
        let Some(start_offset) = self.offset_for_index(start_idx) else {
            return Vec::new();
        };
        let Some(end_offset_inclusive) = self.offset_for_index(end) else {
            return Vec::new();
        };

        let entries = self.log[start_offset..=end_offset_inclusive].to_vec();
        self.last_applied = end;
        entries
    }

    pub fn compact_committed(&mut self) -> bool {
        let target_index = self.commit_index;
        if target_index <= self.snapshot_last_included_index {
            return false;
        }

        let Some(target_term) = self.term_at(target_index) else {
            return false;
        };

        let remove_count = (target_index - self.snapshot_last_included_index).min(self.log.len());
        self.log.drain(0..remove_count);

        self.snapshot_last_included_index = target_index;
        self.snapshot_last_included_term = target_term;
        if self.last_applied < target_index {
            self.last_applied = target_index;
        }

        true
    }

    pub fn restore_from_storage(
        &mut self,
        current_term: Term,
        voted_for: Option<NodeId>,
        log: Vec<LogEntry>,
        commit_index: usize,
        last_included_index: usize,
        last_included_term: Term,
    ) {
        self.current_term = current_term;
        self.voted_for = voted_for;
        self.log = log;
        self.snapshot_last_included_index = last_included_index;
        self.snapshot_last_included_term = last_included_term;
        self.role = Role::Follower;
        self.leader_id = None;
        self.votes_received.clear();
        self.leader_next_index.clear();
        self.leader_match_index.clear();
        self.election_elapsed = 0;
        self.heartbeat_elapsed = 0;

        let max_commit = self.last_log_index();
        self.commit_index = commit_index.clamp(last_included_index, max_commit);
        self.last_applied = last_included_index;
    }

    fn become_follower(&mut self, term: Term, leader: Option<NodeId>) {
        self.role = Role::Follower;
        self.current_term = term;
        self.voted_for = None;
        self.votes_received.clear();
        self.leader_next_index.clear();
        self.leader_match_index.clear();
        self.election_elapsed = 0;
        self.current_election_timeout =
            Self::sample_election_timeout(self.election_timeout_min, self.election_timeout_max);
        self.heartbeat_elapsed = 0;
        self.leader_id = leader;
    }

    fn sample_election_timeout(min_ticks: u64, max_ticks: u64) -> u64 {
        if min_ticks == max_ticks {
            return min_ticks;
        }

        rand::thread_rng().gen_range(min_ticks..=max_ticks)
    }

    fn become_leader(&mut self) {
        self.role = Role::Leader;
        self.leader_id = Some(self.id);
        self.heartbeat_elapsed = 0;
        self.leader_next_index.clear();
        self.leader_match_index.clear();
        self.leader_match_index
            .insert(self.id, self.last_log_index());

        let next = self.last_log_index() + 1;
        for peer in &self.peers {
            self.leader_next_index.insert(*peer, next);
            self.leader_match_index
                .insert(*peer, self.snapshot_last_included_index);
        }
    }

    fn build_heartbeat_messages(&self) -> Vec<OutboundMessage> {
        self.peers
            .iter()
            .copied()
            .filter_map(|peer| {
                self.build_append_entries_for_peer(peer)
                    .map(|message| OutboundMessage::AppendEntries { to: peer, message })
            })
            .collect()
    }

    fn build_append_entries_for_peer(&self, peer: NodeId) -> Option<AppendEntries> {
        let next_idx = self.leader_next_index.get(&peer).copied().unwrap_or(1);
        if next_idx <= self.snapshot_last_included_index {
            return None;
        }

        let prev_log_index = next_idx.saturating_sub(1);
        let prev_log_term = self.term_at(prev_log_index)?;

        let entries = if next_idx == 0 || next_idx > self.last_log_index() {
            Vec::new()
        } else {
            let offset = self.offset_for_index(next_idx)?;
            self.log[offset..].to_vec()
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
        let index = self.last_log_index();
        (index, self.term_at(index).unwrap_or(0))
    }

    fn try_advance_commit_index(&mut self) {
        if self.role != Role::Leader {
            return;
        }

        for idx in (self.commit_index + 1..=self.last_log_index()).rev() {
            if self.term_at(idx) != Some(self.current_term) {
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

    fn last_log_index(&self) -> usize {
        self.snapshot_last_included_index + self.log.len()
    }

    fn offset_for_index(&self, index: usize) -> Option<usize> {
        if index <= self.snapshot_last_included_index {
            return None;
        }

        let offset = index - self.snapshot_last_included_index - 1;
        if offset < self.log.len() {
            Some(offset)
        } else {
            None
        }
    }

    fn term_at(&self, index: usize) -> Option<Term> {
        if index == self.snapshot_last_included_index {
            return Some(self.snapshot_last_included_term);
        }

        self.offset_for_index(index)
            .map(|offset| self.log[offset].term)
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
        RaftNode::new(id, peers, 3, 3, 1)
    }

    #[test]
    fn start_election_votes_for_self_and_sends_requests() {
        let mut n = node(1);
        let messages = n.start_election();

        assert_eq!(n.role, Role::Candidate);
        assert_eq!(n.current_term, 1);
        assert_eq!(n.voted_for, Some(1));
        assert_eq!(messages.len(), 2);
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
    }

    #[test]
    fn request_vote_grants_if_log_is_up_to_date() {
        let mut n = node(2);
        let resp = n.handle_request_vote(RequestVote {
            term: 1,
            candidate_id: 1,
            last_log_index: 0,
            last_log_term: 0,
        });

        assert!(resp.vote_granted);
        assert_eq!(n.voted_for, Some(1));
        assert_eq!(n.current_term, 1);
    }

    #[test]
    fn request_vote_rejects_stale_log() {
        let mut n = node(2);
        n.log.push(LogEntry {
            term: 2,
            command: "x".into(),
        });
        let resp = n.handle_request_vote(RequestVote {
            term: 2,
            candidate_id: 1,
            last_log_index: 0,
            last_log_term: 0,
        });

        assert!(!resp.vote_granted);
    }

    #[test]
    fn append_entries_with_commit_advances_commit_index() {
        let mut follower = node(2);
        let resp = follower.handle_append_entries(AppendEntries {
            term: 1,
            leader_id: 1,
            prev_log_index: 0,
            prev_log_term: 0,
            entries: vec![LogEntry {
                term: 1,
                command: "set a 1".into(),
            }],
            leader_commit: 1,
        });

        assert!(resp.success);
        assert_eq!(follower.log.len(), 1);
        assert_eq!(follower.commit_index, 1);
    }

    #[test]
    fn append_entries_rewrites_conflicting_suffix() {
        let mut follower = node(2);
        follower.log.push(LogEntry {
            term: 1,
            command: "old1".into(),
        });
        follower.log.push(LogEntry {
            term: 2,
            command: "old2".into(),
        });

        let resp = follower.handle_append_entries(AppendEntries {
            term: 3,
            leader_id: 1,
            prev_log_index: 1,
            prev_log_term: 1,
            entries: vec![LogEntry {
                term: 3,
                command: "new2".into(),
            }],
            leader_commit: 2,
        });

        assert!(resp.success);
        assert_eq!(follower.log.len(), 2);
        assert_eq!(follower.log[1].term, 3);
        assert_eq!(follower.log[1].command, "new2");
    }

    #[test]
    fn leader_retries_with_lower_next_index_after_reject() {
        let mut leader = node(1);
        leader.start_election();
        leader.handle_request_vote_response(RequestVoteResponse {
            term: 1,
            vote_granted: true,
            from: 2,
        });
        leader.log.push(LogEntry {
            term: 1,
            command: "cmd1".into(),
        });
        leader.log.push(LogEntry {
            term: 1,
            command: "cmd2".into(),
        });
        leader.leader_next_index.insert(2, 3);

        let outbound = leader.handle_append_entries_response(AppendEntriesResponse {
            term: 1,
            success: false,
            from: 2,
            match_index: 0,
        });

        assert_eq!(leader.leader_next_index.get(&2), Some(&2));
        assert_eq!(outbound.len(), 1);
        match &outbound[0] {
            OutboundMessage::AppendEntries { to, message } => {
                assert_eq!(*to, 2);
                assert_eq!(message.prev_log_index, 1);
                assert_eq!(message.entries.len(), 1);
                assert_eq!(message.entries[0].command, "cmd2");
            }
            _ => panic!("expected append entries retry"),
        }
    }

    #[test]
    fn leader_commits_after_majority_ack() {
        let mut leader = node(1);
        leader.start_election();
        leader.handle_request_vote_response(RequestVoteResponse {
            term: 1,
            vote_granted: true,
            from: 2,
        });

        leader.log.push(LogEntry {
            term: 1,
            command: "cmd1".into(),
        });
        leader.leader_match_index.insert(1, 1);
        leader.leader_match_index.insert(2, 0);
        leader.leader_match_index.insert(3, 0);
        leader.leader_next_index.insert(2, 2);
        leader.leader_next_index.insert(3, 2);

        let outbound = leader.handle_append_entries_response(AppendEntriesResponse {
            term: 1,
            success: true,
            from: 2,
            match_index: 1,
        });

        assert!(outbound.is_empty());
        assert_eq!(leader.commit_index, 1);
    }

    #[test]
    fn commit_makes_entries_available_for_application() {
        let mut n = node(1);
        n.log.push(LogEntry {
            term: 1,
            command: "a".into(),
        });
        n.log.push(LogEntry {
            term: 1,
            command: "b".into(),
        });
        n.commit_to(2);

        let first = n.take_unapplied_entries();
        let second = n.take_unapplied_entries();

        assert_eq!(first.len(), 2);
        assert_eq!(first[0].command, "a");
        assert_eq!(first[1].command, "b");
        assert!(second.is_empty());
    }

    #[test]
    fn leader_proposal_sends_entry_to_followers() {
        let mut leader = node(1);
        leader.start_election();
        assert!(leader.handle_request_vote_response(RequestVoteResponse {
            term: 1,
            vote_granted: true,
            from: 2,
        }));

        let outbound = leader
            .propose_command("set k v")
            .expect("leader should accept proposal");

        assert_eq!(leader.log.len(), 1);
        assert_eq!(leader.log[0].command, "set k v");
        assert_eq!(outbound.len(), 2);

        for msg in outbound {
            match msg {
                OutboundMessage::AppendEntries { to, message } => {
                    assert!(to == 2 || to == 3);
                    assert_eq!(message.entries.len(), 1);
                    assert_eq!(message.entries[0].command, "set k v");
                    assert_eq!(message.prev_log_index, 0);
                }
                _ => panic!("expected append entries"),
            }
        }
    }

    #[test]
    fn compaction_removes_committed_prefix_and_tracks_snapshot_point() {
        let mut n = node(1);
        n.role = Role::Leader;
        n.current_term = 2;

        n.propose_command("set a 1");
        n.propose_command("set b 2");
        n.propose_command("set c 3");
        n.commit_to(2);

        assert!(n.compact_committed());
        assert_eq!(n.snapshot_last_included_index, 2);
        assert_eq!(n.snapshot_last_included_term, 2);
        assert_eq!(n.log.len(), 1);
        assert_eq!(n.commit_index, 2);

        let unapplied = n.take_unapplied_entries();
        assert!(unapplied.is_empty());
    }

    #[test]
    fn restore_from_storage_preserves_snapshot_offsets() {
        let mut n = node(1);
        n.restore_from_storage(
            4,
            Some(2),
            vec![LogEntry {
                term: 4,
                command: "set k v".to_string(),
            }],
            6,
            5,
            3,
        );

        assert_eq!(n.current_term, 4);
        assert_eq!(n.voted_for, Some(2));
        assert_eq!(n.snapshot_last_included_index, 5);
        assert_eq!(n.snapshot_last_included_term, 3);
        assert_eq!(n.commit_index, 6);
        assert_eq!(n.last_applied, 5);
    }
}
