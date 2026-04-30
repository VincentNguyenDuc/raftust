use super::{CommunicationError, InboundMessage, RaftCommunication, RaftMessage, SendOutcome};
use crate::NodeId;

pub struct GrpcCommunication {
    local_id: NodeId,
    started: bool,
}

impl GrpcCommunication {
    pub fn new(local_id: NodeId) -> Self {
        Self {
            local_id,
            started: false,
        }
    }

    pub fn local_id(&self) -> NodeId {
        self.local_id
    }
}

impl RaftCommunication for GrpcCommunication {
    fn start(&mut self, _address: String) -> Result<(), CommunicationError> {
        self.started = true;
        Ok(())
    }

    fn poll(&mut self) -> Result<Option<InboundMessage>, CommunicationError> {
        if !self.started {
            return Err(CommunicationError::NotStarted);
        }

        Ok(None)
    }

    fn send(&mut self, _to: NodeId, _message: RaftMessage) -> SendOutcome {
        if !self.started {
            return SendOutcome::Dropped("communication is not started".to_string());
        }

        SendOutcome::Dropped("gRPC communication scaffold is not implemented yet".to_string())
    }
}
