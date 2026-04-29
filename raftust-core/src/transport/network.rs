use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::thread;

use crate::NodeId;

use super::wire::WireMessage;

pub fn send_to_peer(peer_addrs: &HashMap<NodeId, String>, msg: WireMessage) {
    let addr = match peer_addrs.get(&msg.to) {
        Some(addr) => addr,
        None => {
            eprintln!("no address configured for peer {}", msg.to);
            return;
        }
    };

    if let Err(err) = send_wire(addr, &msg) {
        eprintln!("send error {} -> {} ({}): {}", msg.from, msg.to, addr, err);
    }
}

pub fn spawn_listener(addr: String, tx: Sender<WireMessage>) -> Result<(), String> {
    let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {}: {}", addr, e))?;
    thread::spawn(move || {
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let reader = BufReader::new(stream);
                        for line in reader.lines() {
                            let line = match line {
                                Ok(line) => line,
                                Err(err) => {
                                    eprintln!("read incoming line failed: {}", err);
                                    break;
                                }
                            };
                            match serde_json::from_str::<WireMessage>(&line) {
                                Ok(msg) => {
                                    if tx.send(msg).is_err() {
                                        break;
                                    }
                                }
                                Err(err) => {
                                    eprintln!("invalid wire message: {}", err);
                                }
                            }
                        }
                    });
                }
                Err(err) => {
                    eprintln!("accept failed: {}", err);
                }
            }
        }
    });
    Ok(())
}

pub fn spawn_stdin_reader(tx: Sender<String>) {
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

fn send_wire(addr: &str, msg: &WireMessage) -> Result<(), String> {
    let mut stream = TcpStream::connect(addr).map_err(|e| format!("connect {}: {}", addr, e))?;
    let data = serde_json::to_string(msg).map_err(|e| format!("serialize: {}", e))?;
    stream
        .write_all(data.as_bytes())
        .map_err(|e| format!("write: {}", e))?;
    stream
        .write_all(b"\n")
        .map_err(|e| format!("newline: {}", e))?;
    Ok(())
}
