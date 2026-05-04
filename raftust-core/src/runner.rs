use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::communication::{CommunicationError, RaftCommunication, RaftMessage, SendOutcome};
use crate::config::Config;
use crate::storage::{StorageSnapshot, StorageStrategy};
use crate::{
    AppendEntriesResponse, InstallSnapshotResponse, NodeId, OutboundMessage, RaftNode,
    RequestVoteResponse,
};

#[path = "state_machine.rs"]
pub mod state_machine;
use state_machine::StateMachineStrategy;

#[derive(Debug)]
pub enum Command {
    Propose(String),
    ForceElection,
    Status,
    Shutdown,
}

pub struct Runner<TCommunication, TStorage, TStateMachine>
where
    TCommunication: RaftCommunication,
    TStorage: StorageStrategy,
    TStateMachine: StateMachineStrategy,
{
    config: Config,
    node: RaftNode,
    communication: TCommunication,
    storage: TStorage,
    state_machine: TStateMachine,
}

impl<TCommunication, TStorage, TStateMachine> Runner<TCommunication, TStorage, TStateMachine>
where
    TCommunication: RaftCommunication,
    TStorage: StorageStrategy,
    TStateMachine: StateMachineStrategy,
{
    pub fn new(
        config: Config,
        communication: TCommunication,
        storage: TStorage,
        state_machine: TStateMachine,
    ) -> Self {
        let peers = config.peer_addrs.keys().copied().collect::<Vec<_>>();
        let node = RaftNode::new(
            config.id,
            peers,
            config.election_timeout_min_ticks,
            config.election_timeout_max_ticks,
            config.heartbeat_interval_ticks,
        );

        Self {
            config,
            node,
            communication,
            storage,
            state_machine,
        }
    }

    pub fn run(&mut self, command_rx: Receiver<Command>) -> Result<(), CommunicationError> {
        self.communication.start(self.config.addr.clone())?;

        if let Some(snapshot) = self.storage.load(self.config.id) {
            self.restore_from_snapshot(snapshot);
        }
        self.apply_committed_entries();

        let mut next_tick = Instant::now() + Duration::from_millis(self.config.tick_ms);
        loop {
            if Instant::now() >= next_tick {
                next_tick += Duration::from_millis(self.config.tick_ms);
                let outbound = self.node.tick();
                self.dispatch_outbound(outbound);
                self.persist();
            }

            self.process_communication()?;

            match command_rx.try_recv() {
                Ok(cmd) => {
                    if !self.process_command(cmd) {
                        break;
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {}
            }

            std::thread::sleep(Duration::from_millis(5));
        }

        Ok(())
    }

    fn process_communication(&mut self) -> Result<(), CommunicationError> {
        while let Some(msg) = self.communication.poll()? {
            match msg.message {
                RaftMessage::RequestVote(req) => {
                    let resp = self.node.handle_request_vote(req);
                    self.send_or_log(
                        msg.from,
                        RaftMessage::RequestVoteResponse(RequestVoteResponse {
                            term: resp.term,
                            vote_granted: resp.vote_granted,
                            from: self.node.id,
                        }),
                    );
                    self.apply_committed_entries();
                    self.persist();
                }
                RaftMessage::RequestVoteResponse(resp) => {
                    let became_leader = self.node.handle_request_vote_response(resp);

                    if became_leader {
                        println!(
                            "node {} became leader for term {}",
                            self.node.id, self.node.current_term
                        );
                        let outbound = self.node.tick();
                        self.dispatch_outbound(outbound);
                    }
                    self.apply_committed_entries();
                    self.persist();
                }
                RaftMessage::AppendEntries(req) => {
                    let resp = self.node.handle_append_entries(req);
                    self.send_or_log(
                        msg.from,
                        RaftMessage::AppendEntriesResponse(AppendEntriesResponse {
                            term: resp.term,
                            success: resp.success,
                            from: self.node.id,
                            match_index: resp.match_index,
                        }),
                    );
                    self.apply_committed_entries();
                    self.persist();
                }
                RaftMessage::AppendEntriesResponse(resp) => {
                    let outbound = self.node.handle_append_entries_response(resp);
                    self.dispatch_outbound(outbound);
                    self.apply_committed_entries();
                    self.persist();
                }
                RaftMessage::InstallSnapshot(req) => {
                    if req.term < self.node.current_term {
                        self.send_or_log(
                            msg.from,
                            RaftMessage::InstallSnapshotResponse(InstallSnapshotResponse {
                                term: self.node.current_term,
                                from: self.node.id,
                                success: false,
                            }),
                        );
                        continue;
                    }

                    if let Err(err) = self.state_machine.restore(&req.data) {
                        eprintln!("install snapshot restore error: {}", err);
                        self.send_or_log(
                            msg.from,
                            RaftMessage::InstallSnapshotResponse(InstallSnapshotResponse {
                                term: self.node.current_term,
                                from: self.node.id,
                                success: false,
                            }),
                        );
                        continue;
                    }

                    self.node.restore_from_storage(
                        req.term,
                        None,
                        Vec::new(),
                        req.last_included_index,
                        req.last_included_index,
                        req.last_included_term,
                    );
                    self.node.leader_id = Some(req.leader_id);
                    self.storage.save(StorageSnapshot {
                        node_id: self.node.id,
                        current_term: self.node.current_term,
                        voted_for: self.node.voted_for,
                        log: self.node.log.clone(),
                        commit_index: self.node.commit_index,
                        last_included_index: self.node.snapshot_last_included_index,
                        last_included_term: self.node.snapshot_last_included_term,
                        state_machine_snapshot: req.data,
                    });

                    self.send_or_log(
                        msg.from,
                        RaftMessage::InstallSnapshotResponse(InstallSnapshotResponse {
                            term: self.node.current_term,
                            from: self.node.id,
                            success: true,
                        }),
                    );
                }
                RaftMessage::InstallSnapshotResponse(_resp) => {}
            }
        }

        Ok(())
    }

    fn process_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Shutdown => return false,
            Command::Status => {
                println!(
                    "id={} role={:?} term={} leader={:?} log_len={} snapshot_index={} snapshot_term={} commit_index={} last_applied={} compaction_threshold={} sm={}",
                    self.node.id,
                    self.node.role,
                    self.node.current_term,
                    self.node.leader_id,
                    self.node.log.len(),
                    self.node.snapshot_last_included_index,
                    self.node.snapshot_last_included_term,
                    self.node.commit_index,
                    self.node.last_applied,
                    self.config.log_compaction_threshold,
                    self.state_machine.describe(),
                );
            }
            Command::ForceElection => {
                let outbound = self.node.start_election();
                println!(
                    "node {} started election for term {}",
                    self.node.id, self.node.current_term
                );
                self.dispatch_outbound(outbound);
                self.apply_committed_entries();
                self.persist();
            }
            Command::Propose(value) => match self.node.propose_command(value.clone()) {
                Some(outbound) => {
                    println!("leader {} accepted proposal: {}", self.node.id, value);
                    self.dispatch_outbound(outbound);
                    self.apply_committed_entries();
                    self.persist();
                }
                None => {
                    println!(
                        "node {} is not leader; leader={:?}",
                        self.node.id, self.node.leader_id
                    );
                }
            },
        }
        true
    }

    fn dispatch_outbound(&mut self, outbound: Vec<OutboundMessage>) {
        for outbound_message in outbound {
            match outbound_message {
                OutboundMessage::RequestVote { to, message } => {
                    self.send_or_log(to, RaftMessage::RequestVote(message));
                }
                OutboundMessage::AppendEntries { to, message } => {
                    self.send_or_log(to, RaftMessage::AppendEntries(message));
                }
            }
        }
    }

    fn send_or_log(&mut self, to: NodeId, message: RaftMessage) {
        if let SendOutcome::Dropped(reason) = self.communication.send(to, message) {
            eprintln!(
                "node {} dropped outbound message to {}: {}",
                self.node.id, to, reason
            );
        }
    }

    fn persist(&mut self) {
        self.maybe_compact_log();
        self.storage.save(StorageSnapshot::from_node(
            &self.node,
            self.state_machine.snapshot(),
        ));
    }

    fn maybe_compact_log(&mut self) {
        if self.node.log.len() >= self.config.log_compaction_threshold
            && self.node.compact_committed()
        {
            println!(
                "node {} compacted log at index {}",
                self.node.id, self.node.snapshot_last_included_index
            );
        }
    }

    fn apply_committed_entries(&mut self) {
        for entry in self.node.take_unapplied_entries() {
            self.state_machine.apply(&entry.command);
        }
    }

    fn restore_from_snapshot(&mut self, snapshot: StorageSnapshot) {
        if let Err(err) = self.state_machine.restore(&snapshot.state_machine_snapshot) {
            eprintln!("state machine restore error: {}", err);
        }
        self.node.restore_from_storage(
            snapshot.current_term,
            snapshot.voted_for,
            snapshot.log,
            snapshot.commit_index,
            snapshot.last_included_index,
            snapshot.last_included_term,
        );
    }
}
