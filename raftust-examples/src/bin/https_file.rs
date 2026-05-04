use std::env;
use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::thread;

use log::{error, info, warn};
use raftust_core::config::parse_config;
use raftust_core::runner::{Command, Runner};
use raftust_core_comm_https::HttpsCommunication;
use raftust_core_state_machine_key_value::KeyValueStateMachine;
use raftust_core_storage_file::FileStorage;

fn main() {
    init_logging();
    if let Err(err) = run() {
        error!("fatal: {}", err);
        std::process::exit(1);
    }
}

fn init_logging() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", "info"))
        .format_timestamp_millis()
        .try_init();
}

fn run() -> Result<(), String> {
    let config = parse_config(env::args().skip(1).collect())?;
    let storage_root =
        env::var("RAFTUST_STORAGE_DIR").unwrap_or_else(|_| ".raftust-data".to_string());
    let storage_dir = format!("{}/node-{}", storage_root, config.id);

    info!(
        "example.start node={} addr={} peers={} transport=https storage=file election_timeout_ticks={}..={} heartbeat_ticks={} tick_ms={} compaction_threshold={} storage_dir={}",
        config.id,
        config.addr,
        config.peer_addrs.len(),
        config.election_timeout_min_ticks,
        config.election_timeout_max_ticks,
        config.heartbeat_interval_ticks,
        config.tick_ms,
        config.log_compaction_threshold,
        storage_dir,
    );
    info!("commands: status | election | propose <value> | quit");

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
                    warn!("unknown command; try: status | election | propose <value> | quit")
                }
            }
        }
    });

    let communication = HttpsCommunication::new(config.id, config.peer_addrs.clone());
    let storage = FileStorage::new(storage_dir);
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
