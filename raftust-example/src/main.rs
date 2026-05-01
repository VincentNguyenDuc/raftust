use std::env;

use raftust_core::communication::LocalNetworkCommunication;
use raftust_core::config::parse_config;
use raftust_core::runner::Runner;
use raftust_core::storage::InMemoryStorage;

fn main() {
    if let Err(err) = run() {
        eprintln!("fatal: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = parse_config(env::args().skip(1).collect())?;
    let communication = LocalNetworkCommunication::new(config.id, config.peer_addrs.clone());
    let storage = InMemoryStorage::new();
    let mut runner = Runner::new(config, communication, storage);
    runner.run().map_err(|err| err.to_string())
}
