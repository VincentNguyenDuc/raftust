use std::env;
use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::thread;

use raftust_core::communication::LocalNetworkCommunication;
use raftust_core::config::parse_config;
use raftust_core::runner::{Command, Runner};
use raftust_core::storage::InMemoryStorage;

mod state_machine;
use state_machine::KeyValueStateMachine;

fn main() {
    if let Err(err) = run() {
        eprintln!("fatal: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = parse_config(env::args().skip(1).collect())?;

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

    let (command_tx, command_rx) = mpsc::channel::<Command>();
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let reader = BufReader::new(stdin);
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };
            let cmd = parse_command(line.trim());
            match cmd {
                Some(cmd) => {
                    let shutdown = matches!(cmd, Command::Shutdown);
                    if command_tx.send(cmd).is_err() || shutdown {
                        break;
                    }
                }
                None => {
                    eprintln!("unknown command; try: status | election | propose <value> | quit")
                }
            }
        }
    });

    let communication = LocalNetworkCommunication::new(config.id, config.peer_addrs.clone());
    let storage = InMemoryStorage::new();
    let state_machine = KeyValueStateMachine::new();
    let mut runner = Runner::new(config, communication, storage, state_machine);
    runner.run(command_rx).map_err(|err| err.to_string())
}

fn parse_command(input: &str) -> Option<Command> {
    if input.eq_ignore_ascii_case("quit") || input.eq_ignore_ascii_case("exit") {
        return Some(Command::Shutdown);
    }
    if input.eq_ignore_ascii_case("status") {
        return Some(Command::Status);
    }
    if input.eq_ignore_ascii_case("election") {
        return Some(Command::ForceElection);
    }
    if let Some(value) = input.strip_prefix("propose ") {
        return Some(Command::Propose(value.to_string()));
    }
    None
}
