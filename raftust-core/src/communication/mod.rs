use std::error::Error;
use std::fmt;

use crate::{
    AppendEntries, AppendEntriesResponse, InstallSnapshot, InstallSnapshotResponse, NodeId,
    RequestVote, RequestVoteResponse,
};

pub mod grpc;
pub mod https;
pub mod tcp;
mod wire;

pub use grpc::GrpcCommunication;
pub use https::HttpsCommunication;
pub use tcp::TcpCommunication;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RaftMessage {
    RequestVote(RequestVote),
    RequestVoteResponse(RequestVoteResponse),
    AppendEntries(AppendEntries),
    AppendEntriesResponse(AppendEntriesResponse),
    InstallSnapshot(InstallSnapshot),
    InstallSnapshotResponse(InstallSnapshotResponse),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundMessage {
    pub from: NodeId,
    pub message: RaftMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    Dropped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommunicationError {
    NotStarted,
    Disconnected,
    PeerNotConfigured(NodeId),
    Other(String),
}

impl fmt::Display for CommunicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotStarted => write!(f, "communication is not started"),
            Self::Disconnected => write!(f, "communication channel disconnected"),
            Self::PeerNotConfigured(peer) => write!(f, "peer {} is not configured", peer),
            Self::Other(message) => write!(f, "{}", message),
        }
    }
}

impl Error for CommunicationError {}

pub trait RaftCommunication {
    fn start(&mut self, address: String) -> Result<(), CommunicationError>;
    fn poll(&mut self) -> Result<Option<InboundMessage>, CommunicationError>;
    fn send(&mut self, to: NodeId, message: RaftMessage) -> SendOutcome;
}
