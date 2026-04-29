use std::collections::HashMap;

use raftust_core::NodeId;

#[derive(Debug)]
pub struct Config {
    pub id: NodeId,
    pub addr: String,
    pub peer_addrs: HashMap<NodeId, String>,
    pub election_timeout_ticks: u64,
    pub heartbeat_interval_ticks: u64,
    pub tick_ms: u64,
}

pub fn parse_config(args: Vec<String>) -> Result<Config, String> {
    let mut id: Option<NodeId> = None;
    let mut addr: Option<String> = None;
    let mut peer_addrs = HashMap::new();
    let mut election_timeout_ticks = 20;
    let mut heartbeat_interval_ticks = 4;
    let mut tick_ms = 100;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--id" => {
                let value = args.get(i + 1).ok_or("--id requires a value")?;
                id = Some(value.parse::<NodeId>().map_err(|_| "invalid --id value")?);
                i += 2;
            }
            "--addr" => {
                let value = args.get(i + 1).ok_or("--addr requires a value")?;
                addr = Some(value.clone());
                i += 2;
            }
            "--peer" => {
                let value = args.get(i + 1).ok_or("--peer requires <id>=<addr>")?;
                let (peer_id, peer_addr) = value
                    .split_once('=')
                    .ok_or("--peer must be formatted as <id>=<addr>")?;
                let peer_id = peer_id.parse::<NodeId>().map_err(|_| "invalid peer id")?;
                peer_addrs.insert(peer_id, peer_addr.to_string());
                i += 2;
            }
            "--election-timeout" => {
                let value = args
                    .get(i + 1)
                    .ok_or("--election-timeout requires a value")?;
                election_timeout_ticks = value
                    .parse::<u64>()
                    .map_err(|_| "invalid --election-timeout value")?;
                i += 2;
            }
            "--heartbeat-interval" => {
                let value = args
                    .get(i + 1)
                    .ok_or("--heartbeat-interval requires a value")?;
                heartbeat_interval_ticks = value
                    .parse::<u64>()
                    .map_err(|_| "invalid --heartbeat-interval value")?;
                i += 2;
            }
            "--tick-ms" => {
                let value = args.get(i + 1).ok_or("--tick-ms requires a value")?;
                tick_ms = value
                    .parse::<u64>()
                    .map_err(|_| "invalid --tick-ms value")?;
                i += 2;
            }
            "--help" | "-h" => {
                return Err(help_text());
            }
            unknown => {
                return Err(format!("unknown arg: {}\n\n{}", unknown, help_text()));
            }
        }
    }

    let id = id.ok_or_else(help_text)?;
    let addr = addr.ok_or_else(help_text)?;

    if peer_addrs.contains_key(&id) {
        return Err("--peer cannot include this node's own id".to_string());
    }

    if heartbeat_interval_ticks >= election_timeout_ticks {
        return Err("heartbeat interval must be less than election timeout".to_string());
    }

    Ok(Config {
        id,
        addr,
        peer_addrs,
        election_timeout_ticks,
        heartbeat_interval_ticks,
        tick_ms,
    })
}

fn help_text() -> String {
    [
        "Usage:",
        "  raftust-core --id <node-id> --addr <host:port> --peer <id=host:port> [--peer ...]",
        "",
        "Options:",
        "  --id <u64>                  Node ID",
        "  --addr <host:port>          Listen address for this node",
        "  --peer <id=host:port>       Peer mapping (repeatable)",
        "  --election-timeout <ticks>  Election timeout in ticks (default: 20)",
        "  --heartbeat-interval <ticks> Heartbeat interval in ticks (default: 4)",
        "  --tick-ms <ms>              Tick duration in ms (default: 100)",
        "",
        "Example (3 nodes on localhost):",
        "  raftust-core --id 1 --addr 127.0.0.1:5001 --peer 2=127.0.0.1:5002 --peer 3=127.0.0.1:5003",
    ]
    .join("\n")
}
