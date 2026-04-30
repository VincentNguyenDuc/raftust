use std::env;

use raftust_core::config::parse_config;
use raftust_core::runner::Runner;
use raftust_core::storage::InMemoryStorage;
use raftust_core::transport::NetworkTransport;

fn main() {
    if let Err(err) = run() {
        eprintln!("fatal: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = parse_config(env::args().skip(1).collect())?;
    let transport = NetworkTransport::new(config.peer_addrs.clone());
    let storage = InMemoryStorage::new();
    let mut runner = Runner::new(config, transport, storage);
    runner.run()
}
