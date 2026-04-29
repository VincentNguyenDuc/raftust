use std::env;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use raftust_core::{AppendEntriesResponse, NodeId, OutboundRpc, RaftNode, RequestVoteResponse};

mod app_config;
mod transport;
mod wire;

use app_config::parse_config;
use transport::{send_to_peer, spawn_listener, spawn_stdin_reader};
use wire::{
    WireMessage, WirePayload, from_wire_append_entries, from_wire_request_vote,
    outbound_rpc_to_wire,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("fatal: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = parse_config(env::args().skip(1).collect())?;
    let peers = config.peer_addrs.keys().copied().collect::<Vec<_>>();
    let mut node = RaftNode::new(
        config.id,
        peers,
        config.election_timeout_ticks,
        config.heartbeat_interval_ticks,
    );

    let (network_tx, network_rx) = mpsc::channel::<WireMessage>();
    spawn_listener(config.addr.clone(), network_tx)?;

    let (command_tx, command_rx) = mpsc::channel::<String>();
    spawn_stdin_reader(command_tx);

    println!(
        "node={} addr={} peers={} election_timeout_ticks={} heartbeat_ticks={} tick_ms={}",
        config.id,
        config.addr,
        config.peer_addrs.len(),
        config.election_timeout_ticks,
        config.heartbeat_interval_ticks,
        config.tick_ms
    );
    println!("commands: status | election | propose <value> | quit");

    let mut next_tick = Instant::now() + Duration::from_millis(config.tick_ms);
    loop {
        if Instant::now() >= next_tick {
            next_tick += Duration::from_millis(config.tick_ms);
            let outbound = node.tick();
            dispatch_outbound(node.id, &config.peer_addrs, outbound);
        }

        process_network(&mut node, &config.peer_addrs, &network_rx);

        match command_rx.try_recv() {
            Ok(cmd) => {
                if !process_command(&mut node, &config.peer_addrs, &cmd) {
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

fn process_network(
    node: &mut RaftNode,
    peer_addrs: &std::collections::HashMap<NodeId, String>,
    network_rx: &Receiver<WireMessage>,
) {
    loop {
        let msg = match network_rx.try_recv() {
            Ok(msg) => msg,
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        };

        if msg.to != node.id {
            continue;
        }

        match msg.payload {
            WirePayload::RequestVote {
                term,
                candidate_id,
                last_log_index,
                last_log_term,
            } => {
                let resp = node.handle_request_vote(from_wire_request_vote(
                    term,
                    candidate_id,
                    last_log_index,
                    last_log_term,
                ));

                let outbound = WireMessage {
                    from: node.id,
                    to: msg.from,
                    payload: WirePayload::RequestVoteResponse {
                        term: resp.term,
                        vote_granted: resp.vote_granted,
                    },
                };
                send_to_peer(peer_addrs, outbound);
            }
            WirePayload::RequestVoteResponse { term, vote_granted } => {
                let became_leader = node.handle_request_vote_response(RequestVoteResponse {
                    term,
                    vote_granted,
                    from: msg.from,
                });

                if became_leader {
                    println!(
                        "node {} became leader for term {}",
                        node.id, node.current_term
                    );
                    let outbound = node.tick();
                    dispatch_outbound(node.id, peer_addrs, outbound);
                }
            }
            WirePayload::AppendEntries {
                term,
                leader_id,
                prev_log_index,
                prev_log_term,
                entries,
                leader_commit,
            } => {
                let resp = node.handle_append_entries(from_wire_append_entries(
                    term,
                    leader_id,
                    prev_log_index,
                    prev_log_term,
                    entries,
                    leader_commit,
                ));

                let outbound = WireMessage {
                    from: node.id,
                    to: msg.from,
                    payload: WirePayload::AppendEntriesResponse {
                        term: resp.term,
                        success: resp.success,
                        match_index: resp.match_index,
                    },
                };
                send_to_peer(peer_addrs, outbound);
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
                let outbound = node.handle_append_entries_response(resp);
                if !outbound.is_empty() {
                    dispatch_outbound(node.id, peer_addrs, outbound);
                }
            }
        }
    }
}

fn process_command(
    node: &mut RaftNode,
    peer_addrs: &std::collections::HashMap<NodeId, String>,
    cmd: &str,
) -> bool {
    let cmd = cmd.trim();
    if cmd.eq_ignore_ascii_case("quit") || cmd.eq_ignore_ascii_case("exit") {
        return false;
    }

    if cmd.eq_ignore_ascii_case("status") {
        println!(
            "id={} role={:?} term={} leader={:?} log_len={} commit_index={} last_applied={} sm={:?}",
            node.id,
            node.role,
            node.current_term,
            node.leader_id,
            node.log.len(),
            node.commit_index,
            node.last_applied,
            node.state_machine,
        );
        return true;
    }

    if cmd.eq_ignore_ascii_case("election") {
        let outbound = node.start_election();
        println!(
            "node {} started election for term {}",
            node.id, node.current_term
        );
        dispatch_outbound(node.id, peer_addrs, outbound);
        return true;
    }

    if let Some(value) = cmd.strip_prefix("propose ") {
        match node.propose_command(value.to_string()) {
            Some(outbound) => {
                println!("leader {} accepted proposal: {}", node.id, value);
                dispatch_outbound(node.id, peer_addrs, outbound);
            }
            None => {
                println!(
                    "node {} is not leader; leader={:?}",
                    node.id, node.leader_id
                );
            }
        }
        return true;
    }

    println!("unknown command: {}", cmd);
    true
}

fn dispatch_outbound(
    from: NodeId,
    peer_addrs: &std::collections::HashMap<NodeId, String>,
    outbound: Vec<OutboundRpc>,
) {
    for rpc in outbound {
        send_to_peer(peer_addrs, outbound_rpc_to_wire(from, rpc));
    }
}
