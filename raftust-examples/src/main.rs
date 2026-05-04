fn main() {
    println!("raftust-examples");
    println!();
    println!("This package contains multiple runnable example binaries.");
    println!("Run one of the following:");
    println!();
    println!(
        "  cargo run -p raftust-examples --bin grpc_in_memory -- --id <id> --addr <host:port> --peer <id=host:port> [--peer ...]"
    );
    println!(
        "  cargo run -p raftust-examples --bin https_file -- --id <id> --addr <host:port> --peer <id=host:port> [--peer ...]"
    );
    println!();
    println!("Optional for https_file:");
    println!("  RAFTUST_STORAGE_DIR=<path>   Base directory for persisted snapshots");
    println!();
    println!("Common runtime commands once a node starts:");
    println!("  status");
    println!("  election");
    println!("  propose <value>");
    println!("  quit");
}
