use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::storage::{StorageSnapshot, StorageStrategy};
use crate::transport::network::spawn_stdin_reader;
use crate::transport::wire::{
    WireMessage, WirePayload, from_wire_append_entries, from_wire_request_vote,
    outbound_rpc_to_wire,
};
use crate::{AppendEntriesResponse, OutboundRpc, RaftNode, RequestVoteResponse, TransportStrategy};

pub struct Runner<TTransport, TStorage>
where
    TTransport: TransportStrategy,
    TStorage: StorageStrategy,
{
    config: Config,
    node: RaftNode,
    transport: TTransport,
    storage: TStorage,
}

impl<TTransport, TStorage> Runner<TTransport, TStorage>
where
    TTransport: TransportStrategy,
    TStorage: StorageStrategy,
{
    pub fn new(config: Config, transport: TTransport, storage: TStorage) -> Self {
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
            transport,
            storage,
        }
    }

    pub fn run(&mut self) -> Result<(), String> {
        self.transport.start_listener(self.config.addr.clone())?;

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

            self.process_network();

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

    fn process_network(&mut self) {
        while let Some(msg) = self.transport.try_recv() {
            if msg.to != self.node.id {
                continue;
            }

            match msg.payload {
                WirePayload::RequestVote {
                    term,
                    candidate_id,
                    last_log_index,
                    last_log_term,
                } => {
                    let resp = self.node.handle_request_vote(from_wire_request_vote(
                        term,
                        candidate_id,
                        last_log_index,
                        last_log_term,
                    ));

                    let outbound = WireMessage {
                        from: self.node.id,
                        to: msg.from,
                        payload: WirePayload::RequestVoteResponse {
                            term: resp.term,
                            vote_granted: resp.vote_granted,
                        },
                    };
                    self.transport.send(outbound);
                    self.persist();
                }
                WirePayload::RequestVoteResponse { term, vote_granted } => {
                    let became_leader =
                        self.node.handle_request_vote_response(RequestVoteResponse {
                            term,
                            vote_granted,
                            from: msg.from,
                        });

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
                WirePayload::AppendEntries {
                    term,
                    leader_id,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit,
                } => {
                    let resp = self.node.handle_append_entries(from_wire_append_entries(
                        term,
                        leader_id,
                        prev_log_index,
                        prev_log_term,
                        entries,
                        leader_commit,
                    ));

                    let outbound = WireMessage {
                        from: self.node.id,
                        to: msg.from,
                        payload: WirePayload::AppendEntriesResponse {
                            term: resp.term,
                            success: resp.success,
                            match_index: resp.match_index,
                        },
                    };
                    self.transport.send(outbound);
                    self.persist();
                }
                WirePayload::AppendEntriesResponse {
                    term,
                    success,
                    match_index,
                } => {
                    let resp = AppendEntriesResponse {
                        term,
                        success,
                        from: msg.from,
                        match_index,
                    };
                    let outbound = self.node.handle_append_entries_response(resp);
                    self.dispatch_outbound(outbound);
                    self.persist();
                }
            }
        }
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

    fn dispatch_outbound(&mut self, outbound: Vec<OutboundRpc>) {
        for rpc in outbound {
            self.transport.send(outbound_rpc_to_wire(self.node.id, rpc));
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
