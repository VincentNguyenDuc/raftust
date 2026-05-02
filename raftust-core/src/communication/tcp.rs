use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use super::wire::{self, WireMessage};
use super::{CommunicationError, InboundMessage, RaftCommunication, RaftMessage, SendOutcome};
use crate::NodeId;

pub struct TcpCommunication {
    local_id: NodeId,
    peer_addrs: HashMap<NodeId, String>,
    inbound_rx: Option<Receiver<WireMessage>>,
}

impl TcpCommunication {
    pub fn new(local_id: NodeId, peer_addrs: HashMap<NodeId, String>) -> Self {
        Self {
            local_id,
            peer_addrs,
            inbound_rx: None,
        }
    }

    fn send_to_peer(&self, msg: WireMessage) -> SendOutcome {
        let addr = match self.peer_addrs.get(&msg.to) {
            Some(addr) => addr,
            None => {
                return SendOutcome::Dropped(format!("peer {} is not configured", msg.to));
            }
        };

        if let Err(err) = Self::send_wire(addr, &msg) {
            return SendOutcome::Dropped(format!(
                "send error {} -> {} ({}): {}",
                msg.from, msg.to, addr, err
            ));
        }

        SendOutcome::Sent
    }

    fn spawn_listener(addr: String, tx: Sender<WireMessage>) -> Result<(), CommunicationError> {
        let listener = TcpListener::bind(&addr)
            .map_err(|e| CommunicationError::Other(format!("bind {}: {}", addr, e)))?;
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

    fn send_wire(addr: &str, msg: &WireMessage) -> Result<(), CommunicationError> {
        let mut stream = TcpStream::connect(addr)
            .map_err(|e| CommunicationError::Other(format!("connect {}: {}", addr, e)))?;
        let data = serde_json::to_string(msg)
            .map_err(|e| CommunicationError::Other(format!("serialize: {}", e)))?;
        stream
            .write_all(data.as_bytes())
            .map_err(|e| CommunicationError::Other(format!("write: {}", e)))?;
        stream
            .write_all(b"\n")
            .map_err(|e| CommunicationError::Other(format!("newline: {}", e)))?;
        Ok(())
    }
}

impl RaftCommunication for TcpCommunication {
    fn start(&mut self, address: String) -> Result<(), CommunicationError> {
        let (tx, rx) = mpsc::channel::<WireMessage>();
        Self::spawn_listener(address, tx)?;
        self.inbound_rx = Some(rx);
        Ok(())
    }

    fn poll(&mut self) -> Result<Option<InboundMessage>, CommunicationError> {
        let rx = self
            .inbound_rx
            .as_ref()
            .ok_or(CommunicationError::NotStarted)?;
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    if msg.to != self.local_id {
                        continue;
                    }
                    return Ok(Some(wire::wire_to_message(msg)));
                }
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => return Err(CommunicationError::Disconnected),
            }
        }
    }

    fn send(&mut self, to: NodeId, message: RaftMessage) -> SendOutcome {
        let msg = wire::message_to_wire(self.local_id, to, message);
        self.send_to_peer(msg)
    }
}
