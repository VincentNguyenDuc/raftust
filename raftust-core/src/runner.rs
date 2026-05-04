use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::communication::{CommunicationError, RaftCommunication, RaftMessage, SendOutcome};
use crate::config::Config;
use crate::storage::{StorageSnapshot, StorageStrategy};
use crate::{
    AppendEntriesResponse, InstallSnapshotResponse, NodeId, OutboundMessage, RaftNode,
    RequestVoteResponse,
};
use log::{debug, error, info, warn};

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
    inbound_count_since_report: u64,
    applied_count_since_report: u64,
    dropped_outbound_since_report: u64,
    last_report_at: Instant,
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
            inbound_count_since_report: 0,
            applied_count_since_report: 0,
            dropped_outbound_since_report: 0,
            last_report_at: Instant::now(),
        }
    }

    pub fn run(&mut self, command_rx: Receiver<Command>) -> Result<(), CommunicationError> {
        info!(
            "event=runner_start node_id={} addr={} peer_count={} tick_ms={} election_timeout_min_ticks={} election_timeout_max_ticks={} heartbeat_interval_ticks={} log_compaction_threshold={}",
            self.config.id,
            self.config.addr,
            self.config.peer_addrs.len(),
            self.config.tick_ms,
            self.config.election_timeout_min_ticks,
            self.config.election_timeout_max_ticks,
            self.config.heartbeat_interval_ticks,
            self.config.log_compaction_threshold
        );
        self.communication.start(self.config.addr.clone())?;

        if let Some(snapshot) = self.storage.load(self.config.id) {
            info!(
                "event=runner_restore node_id={} term={} log_len={} snapshot_index={} snapshot_term={} commit_index={}",
                self.node.id,
                snapshot.current_term,
                snapshot.log.len(),
                snapshot.last_included_index,
                snapshot.last_included_term,
                snapshot.commit_index
            );
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

            self.maybe_report_runtime_metrics();

            std::thread::sleep(Duration::from_millis(5));
        }

        Ok(())
    }

    fn process_communication(&mut self) -> Result<(), CommunicationError> {
        while let Some(msg) = self.communication.poll()? {
            self.inbound_count_since_report += 1;
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
                        info!(
                            "event=leader_elected node_id={} term={}",
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
                        warn!(
                            "event=snapshot_reject_stale node_id={} peer_id={} req_term={} current_term={}",
                            self.node.id, msg.from, req.term, self.node.current_term
                        );
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
                        error!(
                            "event=snapshot_restore_failed node_id={} peer_id={} err={}",
                            self.node.id, msg.from, err
                        );
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
                    info!(
                        "event=snapshot_applied node_id={} leader_id={} snapshot_index={} snapshot_term={} bytes={}",
                        self.node.id,
                        req.leader_id,
                        self.node.snapshot_last_included_index,
                        self.node.snapshot_last_included_term,
                        req.data.len()
                    );
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
                RaftMessage::InstallSnapshotResponse(resp) => {
                    debug!(
                        "event=snapshot_response node_id={} peer_id={} term={} success={}",
                        self.node.id, resp.from, resp.term, resp.success
                    );
                }
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
                info!(
                    "event=force_election node_id={} term={}",
                    self.node.id, self.node.current_term
                );
                self.dispatch_outbound(outbound);
                self.apply_committed_entries();
                self.persist();
            }
            Command::Propose(value) => match self.node.propose_command(value.clone()) {
                Some(outbound) => {
                    info!(
                        "event=proposal_accepted node_id={} term={} value={}",
                        self.node.id, self.node.current_term, value
                    );
                    self.dispatch_outbound(outbound);
                    self.apply_committed_entries();
                    self.persist();
                }
                None => {
                    warn!(
                        "event=proposal_rejected_not_leader node_id={} leader_id={:?}",
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
            self.dropped_outbound_since_report += 1;
            warn!(
                "event=outbound_dropped node_id={} peer_id={} reason={}",
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
            info!(
                "event=compaction node_id={} snapshot_index={} snapshot_term={} remaining_log_len={}",
                self.node.id,
                self.node.snapshot_last_included_index,
                self.node.snapshot_last_included_term,
                self.node.log.len()
            );
        }
    }

    fn apply_committed_entries(&mut self) {
        for entry in self.node.take_unapplied_entries() {
            self.applied_count_since_report += 1;
            self.state_machine.apply(&entry.command);
        }
    }

    fn maybe_report_runtime_metrics(&mut self) {
        let report_interval =
            Duration::from_millis(self.config.report_metrics_interval_ticks * self.config.tick_ms);
        if self.last_report_at.elapsed() < report_interval {
            return;
        }

        info!(
            "event=runtime_metrics node_id={} term={} role={:?} inbound_count={} applied_count={} outbound_dropped_count={} log_len={} commit_index={} last_applied={} snapshot_index={}",
            self.node.id,
            self.node.current_term,
            self.node.role,
            self.inbound_count_since_report,
            self.applied_count_since_report,
            self.dropped_outbound_since_report,
            self.node.log.len(),
            self.node.commit_index,
            self.node.last_applied,
            self.node.snapshot_last_included_index
        );

        self.inbound_count_since_report = 0;
        self.applied_count_since_report = 0;
        self.dropped_outbound_since_report = 0;
        self.last_report_at = Instant::now();
    }

    fn restore_from_snapshot(&mut self, snapshot: StorageSnapshot) {
        if let Err(err) = self.state_machine.restore(&snapshot.state_machine_snapshot) {
            error!(
                "event=state_machine_restore_failed node_id={} err={}",
                self.node.id, err
            );
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
