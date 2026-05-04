use crate::{LogEntry, NodeId, RaftNode, Term};

#[derive(Debug, Clone)]
pub struct StorageSnapshot {
    pub node_id: NodeId,
    pub current_term: Term,
    pub voted_for: Option<NodeId>,
    pub log: Vec<LogEntry>,
    pub commit_index: usize,
    pub last_included_index: usize,
    pub last_included_term: Term,
    pub state_machine_snapshot: Vec<u8>,
}

impl StorageSnapshot {
    pub fn from_node(node: &RaftNode, state_machine_snapshot: Vec<u8>) -> Self {
        Self {
            node_id: node.id,
            current_term: node.current_term,
            voted_for: node.voted_for,
            log: node.log.clone(),
            commit_index: node.commit_index,
            last_included_index: node.snapshot_last_included_index,
            last_included_term: node.snapshot_last_included_term,
            state_machine_snapshot,
        }
    }
}

pub trait StorageStrategy {
    fn load(&self, _node_id: NodeId) -> Option<StorageSnapshot> {
        None
    }

    fn save(&mut self, snapshot: StorageSnapshot);
}
