use std::io::{BufRead, BufReader};
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::communication::{CommunicationError, RaftCommunication, RaftMessage, SendOutcome};
use crate::config::Config;
use crate::storage::{StorageSnapshot, StorageStrategy};
use crate::{
    AppendEntriesResponse, InstallSnapshotResponse, NodeId, OutboundMessage, RaftNode,
    RequestVoteResponse,
};

pub struct Runner<TCommunication, TStorage>
where
    TCommunication: RaftCommunication,
    TStorage: StorageStrategy,
{
    config: Config,
    node: RaftNode,
    communication: TCommunication,
    storage: TStorage,
}

impl<TCommunication, TStorage> Runner<TCommunication, TStorage>
where
    TCommunication: RaftCommunication,
    TStorage: StorageStrategy,
{
    pub fn new(config: Config, communication: TCommunication, storage: TStorage) -> Self {
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
        }
    }

    pub fn run(&mut self) -> Result<(), CommunicationError> {
        self.communication.start(self.config.addr.clone())?;

        if let Some(snapshot) = self.storage.load(self.config.id) {
            self.restore_from_snapshot(snapshot);
        }

        let (command_tx, command_rx) = mpsc::channel::<String>();
        spawn_stdin_reader(command_tx);

        println!(
            "node={} addr={} peers={} election_timeout_ticks={} heartbeat_ticks={} tick_ms={}",
            self.config.id,
            self.config.addr,
            self.config.peer_addrs.len(),
            self.config.election_timeout_ticks,
            self.config.heartbeat_interval_ticks,
            self.config.tick_ms
        );
        println!("commands: status | election | propose <value> | quit");

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
                    if !self.process_command(&cmd) {
                        break;
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => break,
            }

            thread::sleep(Duration::from_millis(5));
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
                    self.persist();
                }
                RaftMessage::AppendEntriesResponse(resp) => {
                    let outbound = self.node.handle_append_entries_response(resp);
                    self.dispatch_outbound(outbound);
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

    fn process_command(&mut self, cmd: &str) -> bool {
        let cmd = cmd.trim();
        if cmd.eq_ignore_ascii_case("quit") || cmd.eq_ignore_ascii_case("exit") {
            return false;
        }

        if cmd.eq_ignore_ascii_case("status") {
            println!(
                "id={} role={:?} term={} leader={:?} log_len={} commit_index={} last_applied={} sm={:?}",
                self.node.id,
                self.node.role,
                self.node.current_term,
                self.node.leader_id,
                self.node.log.len(),
                self.node.commit_index,
                self.node.last_applied,
                self.node.state_machine,
            );
            return true;
        }

        if cmd.eq_ignore_ascii_case("election") {
            let outbound = self.node.start_election();
            println!(
                "node {} started election for term {}",
                self.node.id, self.node.current_term
            );
            self.dispatch_outbound(outbound);
            self.persist();
            return true;
        }

        if let Some(value) = cmd.strip_prefix("propose ") {
            match self.node.propose_command(value.to_string()) {
                Some(outbound) => {
                    println!("leader {} accepted proposal: {}", self.node.id, value);
                    self.dispatch_outbound(outbound);
                    self.persist();
                }
                None => {
                    println!(
                        "node {} is not leader; leader={:?}",
                        self.node.id, self.node.leader_id
                    );
                }
            }
            return true;
        }

        println!("unknown command: {}", cmd);
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

    fn restore_from_snapshot(&mut self, snapshot: StorageSnapshot) {
        self.node.current_term = snapshot.current_term;
        self.node.voted_for = snapshot.voted_for;
        self.node.log = snapshot.log;
        self.node.commit_index = snapshot.commit_index.min(self.node.log.len());
        self.node.last_applied = snapshot.last_applied.min(self.node.commit_index);
        self.node.state_machine = snapshot.state_machine;
    }
}

fn spawn_stdin_reader(tx: Sender<String>) {
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let reader = BufReader::new(stdin);
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}
