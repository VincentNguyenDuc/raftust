use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::communication::{CommunicationError, RaftCommunication, RaftMessage, SendOutcome};
use crate::config::Config;
use crate::storage::{StorageSnapshot, StorageStrategy};
use crate::{
    AppendEntriesResponse, InstallSnapshotResponse, NodeId, OutboundMessage, RaftNode,
    RequestVoteResponse,
};

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
            config.election_timeout_ticks,
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
                // In non-interactive environments (e.g. containers), command input can be absent.
                // Keep the Raft node running even if the command channel closes.
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
                RaftMessage::InstallSnapshot(_req) => {
                    self.send_or_log(
                        msg.from,
                        RaftMessage::InstallSnapshotResponse(InstallSnapshotResponse {
                            term: self.node.current_term,
                            from: self.node.id,
                            success: false,
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
                    "id={} role={:?} term={} leader={:?} log_len={} commit_index={} last_applied={} sm={}",
                    self.node.id,
                    self.node.role,
                    self.node.current_term,
                    self.node.leader_id,
                    self.node.log.len(),
                    self.node.commit_index,
                    self.node.last_applied,
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
        self.storage.save(StorageSnapshot::from_node(&self.node));
    }

    fn apply_committed_entries(&mut self) {
        for entry in self.node.take_unapplied_entries() {
            self.state_machine.apply(&entry.command);
        }
    }

    fn restore_from_snapshot(&mut self, snapshot: StorageSnapshot) {
        self.node.current_term = snapshot.current_term;
        self.node.voted_for = snapshot.voted_for;
        self.node.log = snapshot.log;
        self.node.commit_index = snapshot.commit_index.min(self.node.log.len());
        self.node.last_applied = 0;
    }
}
