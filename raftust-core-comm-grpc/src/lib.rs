use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread;

use log::{debug, error, info, warn};
use raftust_core::{
    AppendEntries, AppendEntriesResponse, CommunicationError, InboundMessage, InstallSnapshot,
    InstallSnapshotResponse, LogEntry, NodeId, RaftCommunication, RaftMessage, RequestVote,
    RequestVoteResponse, SendOutcome,
};
use tokio::runtime::{Builder, Runtime};
use tonic::{Request, Response, Status, transport::Server};

pub mod proto {
    tonic::include_proto!("raftust.transport.v1");
}

use proto::raft_transport_client::RaftTransportClient;
use proto::raft_transport_server::{RaftTransport, RaftTransportServer};
use proto::transport_envelope::Payload;
use proto::{TransportAck, TransportEnvelope};

pub struct GrpcCommunication {
    local_id: NodeId,
    peer_endpoints: HashMap<NodeId, String>,
    inbound_rx: Option<Receiver<TransportEnvelope>>,
    client_runtime: Option<Runtime>,
}

impl GrpcCommunication {
    pub fn new(local_id: NodeId, peer_addrs: HashMap<NodeId, String>) -> Self {
        let peer_endpoints = peer_addrs
            .into_iter()
            .map(|(id, addr)| (id, format!("http://{}", addr)))
            .collect();

        Self {
            local_id,
            peer_endpoints,
            inbound_rx: None,
            client_runtime: None,
        }
    }

    fn spawn_listener(
        addr: SocketAddr,
        tx: SyncSender<TransportEnvelope>,
    ) -> Result<(), CommunicationError> {
        thread::Builder::new()
            .name(format!("raftust-grpc-{}", addr.port()))
            .spawn(move || {
                let runtime = Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build gRPC server runtime");

                runtime.block_on(async move {
                    info!("event=grpc_listener_start addr={}", addr);
                    let service = GrpcTransportService { tx };
                    if let Err(err) = Server::builder()
                        .add_service(RaftTransportServer::new(service))
                        .serve(addr)
                        .await
                    {
                        error!("event=grpc_listener_error addr={} err={}", addr, err);
                    }
                });
            })
            .map(|_| ())
            .map_err(|err| CommunicationError::Other(format!("spawn listener: {}", err)))
    }
}

impl RaftCommunication for GrpcCommunication {
    fn start(&mut self, address: String) -> Result<(), CommunicationError> {
        let socket_addr: SocketAddr = address
            .parse()
            .map_err(|err| CommunicationError::Other(format!("parse listen address: {}", err)))?;

        let (tx, rx) = mpsc::sync_channel::<TransportEnvelope>(256);
        Self::spawn_listener(socket_addr, tx)?;

        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| CommunicationError::Other(format!("build client runtime: {}", err)))?;

        self.inbound_rx = Some(rx);
        self.client_runtime = Some(runtime);
        info!(
            "event=grpc_communication_started node_id={} address={} peer_count={}",
            self.local_id,
            address,
            self.peer_endpoints.len()
        );
        Ok(())
    }

    fn poll(&mut self) -> Result<Option<InboundMessage>, CommunicationError> {
        let rx = self
            .inbound_rx
            .as_ref()
            .ok_or(CommunicationError::NotStarted)?;

        loop {
            match rx.try_recv() {
                Ok(envelope) => {
                    if envelope.to != self.local_id {
                        debug!(
                            "event=grpc_poll_skip_wrong_recipient node_id={} envelope_to={} peer_id={}",
                            self.local_id, envelope.to, envelope.from
                        );
                        continue;
                    }

                    debug!(
                        "event=grpc_poll_inbound node_id={} peer_id={}",
                        self.local_id, envelope.from
                    );
                    return decode_envelope(envelope).map(Some);
                }
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => return Err(CommunicationError::Disconnected),
            }
        }
    }

    fn send(&mut self, to: NodeId, message: RaftMessage) -> SendOutcome {
        let endpoint = match self.peer_endpoints.get(&to) {
            Some(endpoint) => endpoint.clone(),
            None => {
                warn!(
                    "event=grpc_send_drop_unconfigured_peer node_id={} peer_id={}",
                    self.local_id, to
                );
                return SendOutcome::Dropped(format!("peer {} is not configured", to));
            }
        };

        let runtime = match self.client_runtime.as_ref() {
            Some(runtime) => runtime,
            None => {
                warn!(
                    "event=grpc_send_drop_not_started node_id={} peer_id={}",
                    self.local_id, to
                );
                return SendOutcome::Dropped("communication is not started".to_string());
            }
        };

        let request = match encode_message(self.local_id, to, message) {
            Ok(request) => request,
            Err(err) => return SendOutcome::Dropped(err.to_string()),
        };

        let endpoint_for_request = endpoint.clone();
        let result = runtime.block_on(async move {
            let mut client = RaftTransportClient::connect(endpoint_for_request.clone())
                .await
                .map_err(|err| format!("connect {}: {}", endpoint_for_request, err))?;

            client
                .send_message(Request::new(request))
                .await
                .map(|response| response.into_inner())
                .map_err(|err| format!("send {}: {}", endpoint_for_request, err))
        });

        match result {
            Ok(ack) if ack.accepted => {
                debug!(
                    "event=grpc_send_ok node_id={} peer_id={} endpoint={}",
                    self.local_id, to, endpoint
                );
                SendOutcome::Sent
            }
            Ok(ack) => SendOutcome::Dropped(match ack.error.is_empty() {
                true => format!("peer {} rejected the message", to),
                false => ack.error,
            }),
            Err(err) => {
                warn!(
                    "event=grpc_send_drop_transport node_id={} peer_id={} endpoint={} err={}",
                    self.local_id, to, endpoint, err
                );
                SendOutcome::Dropped(err)
            }
        }
    }
}

struct GrpcTransportService {
    tx: SyncSender<TransportEnvelope>,
}

#[tonic::async_trait]
impl RaftTransport for GrpcTransportService {
    async fn send_message(
        &self,
        request: Request<TransportEnvelope>,
    ) -> Result<Response<TransportAck>, Status> {
        match self.tx.try_send(request.into_inner()) {
            Ok(()) => Ok(Response::new(TransportAck {
                accepted: true,
                error: String::new(),
            })),
            Err(err) => {
                warn!("event=grpc_listener_queue_full err={}", err);
                Ok(Response::new(TransportAck {
                    accepted: false,
                    error: format!("queue inbound message: {}", err),
                }))
            }
        }
    }
}

fn encode_message(
    from: NodeId,
    to: NodeId,
    message: RaftMessage,
) -> Result<TransportEnvelope, CommunicationError> {
    let payload = match message {
        RaftMessage::RequestVote(message) => Payload::RequestVote(proto::RequestVote {
            term: message.term,
            candidate_id: message.candidate_id,
            last_log_index: usize_to_u64(message.last_log_index, "request_vote.last_log_index")?,
            last_log_term: message.last_log_term,
        }),
        RaftMessage::RequestVoteResponse(message) => {
            Payload::RequestVoteResponse(proto::RequestVoteResponse {
                term: message.term,
                vote_granted: message.vote_granted,
            })
        }
        RaftMessage::AppendEntries(message) => Payload::AppendEntries(proto::AppendEntries {
            term: message.term,
            leader_id: message.leader_id,
            prev_log_index: usize_to_u64(message.prev_log_index, "append_entries.prev_log_index")?,
            prev_log_term: message.prev_log_term,
            entries: message
                .entries
                .into_iter()
                .map(|entry| proto::LogEntry {
                    term: entry.term,
                    command: entry.command,
                })
                .collect(),
            leader_commit: usize_to_u64(message.leader_commit, "append_entries.leader_commit")?,
        }),
        RaftMessage::AppendEntriesResponse(message) => {
            Payload::AppendEntriesResponse(proto::AppendEntriesResponse {
                term: message.term,
                success: message.success,
                match_index: usize_to_u64(
                    message.match_index,
                    "append_entries_response.match_index",
                )?,
            })
        }
        RaftMessage::InstallSnapshot(message) => Payload::InstallSnapshot(proto::InstallSnapshot {
            term: message.term,
            leader_id: message.leader_id,
            last_included_index: usize_to_u64(
                message.last_included_index,
                "install_snapshot.last_included_index",
            )?,
            last_included_term: message.last_included_term,
            data: message.data,
        }),
        RaftMessage::InstallSnapshotResponse(message) => {
            Payload::InstallSnapshotResponse(proto::InstallSnapshotResponse {
                term: message.term,
                success: message.success,
            })
        }
    };

    Ok(TransportEnvelope {
        from,
        to,
        payload: Some(payload),
    })
}

fn decode_envelope(envelope: TransportEnvelope) -> Result<InboundMessage, CommunicationError> {
    let from = envelope.from;
    let payload = envelope
        .payload
        .ok_or_else(|| CommunicationError::Other("missing gRPC payload".to_string()))?;

    let message = match payload {
        Payload::RequestVote(message) => RaftMessage::RequestVote(RequestVote {
            term: message.term,
            candidate_id: message.candidate_id,
            last_log_index: u64_to_usize(message.last_log_index, "request_vote.last_log_index")?,
            last_log_term: message.last_log_term,
        }),
        Payload::RequestVoteResponse(message) => {
            RaftMessage::RequestVoteResponse(RequestVoteResponse {
                term: message.term,
                vote_granted: message.vote_granted,
                from,
            })
        }
        Payload::AppendEntries(message) => RaftMessage::AppendEntries(AppendEntries {
            term: message.term,
            leader_id: message.leader_id,
            prev_log_index: u64_to_usize(message.prev_log_index, "append_entries.prev_log_index")?,
            prev_log_term: message.prev_log_term,
            entries: message
                .entries
                .into_iter()
                .map(|entry| LogEntry {
                    term: entry.term,
                    command: entry.command,
                })
                .collect(),
            leader_commit: u64_to_usize(message.leader_commit, "append_entries.leader_commit")?,
        }),
        Payload::AppendEntriesResponse(message) => {
            RaftMessage::AppendEntriesResponse(AppendEntriesResponse {
                term: message.term,
                success: message.success,
                from,
                match_index: u64_to_usize(
                    message.match_index,
                    "append_entries_response.match_index",
                )?,
            })
        }
        Payload::InstallSnapshot(message) => RaftMessage::InstallSnapshot(InstallSnapshot {
            term: message.term,
            leader_id: message.leader_id,
            last_included_index: u64_to_usize(
                message.last_included_index,
                "install_snapshot.last_included_index",
            )?,
            last_included_term: message.last_included_term,
            data: message.data,
        }),
        Payload::InstallSnapshotResponse(message) => {
            RaftMessage::InstallSnapshotResponse(InstallSnapshotResponse {
                term: message.term,
                from,
                success: message.success,
            })
        }
    };

    Ok(InboundMessage { from, message })
}

fn usize_to_u64(value: usize, field: &str) -> Result<u64, CommunicationError> {
    value
        .try_into()
        .map_err(|_| CommunicationError::Other(format!("{} does not fit into u64", field)))
}

fn u64_to_usize(value: u64, field: &str) -> Result<usize, CommunicationError> {
    value
        .try_into()
        .map_err(|_| CommunicationError::Other(format!("{} does not fit into usize", field)))
}

#[cfg(test)]
mod tests {
    use super::{decode_envelope, encode_message};
    use raftust_core::{AppendEntriesResponse, InstallSnapshotResponse, RaftMessage};

    #[test]
    fn request_response_round_trip_preserves_sender() {
        let envelope = encode_message(
            3,
            1,
            RaftMessage::AppendEntriesResponse(AppendEntriesResponse {
                term: 7,
                success: true,
                from: 99,
                match_index: 42,
            }),
        )
        .expect("encode message");

        let inbound = decode_envelope(envelope).expect("decode message");
        match inbound.message {
            RaftMessage::AppendEntriesResponse(response) => {
                assert_eq!(response.from, 3);
                assert_eq!(response.match_index, 42);
                assert!(response.success);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn snapshot_response_round_trip_preserves_sender() {
        let envelope = encode_message(
            5,
            2,
            RaftMessage::InstallSnapshotResponse(InstallSnapshotResponse {
                term: 11,
                from: 21,
                success: false,
            }),
        )
        .expect("encode message");

        let inbound = decode_envelope(envelope).expect("decode message");
        match inbound.message {
            RaftMessage::InstallSnapshotResponse(response) => {
                assert_eq!(response.from, 5);
                assert!(!response.success);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }
}
