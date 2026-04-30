use std::collections::HashMap;

use crate::{LogEntry, NodeId, RaftNode, Term};

#[derive(Debug, Clone)]
pub struct StorageSnapshot {
    pub node_id: NodeId,
    pub current_term: Term,
    pub voted_for: Option<NodeId>,
    pub log: Vec<LogEntry>,
    pub commit_index: usize,
    pub last_applied: usize,
    pub state_machine: HashMap<String, String>,
}

impl StorageSnapshot {
    pub fn from_node(node: &RaftNode) -> Self {
        Self {
            node_id: node.id,
            current_term: node.current_term,
            voted_for: node.voted_for,
            log: node.log.clone(),
            commit_index: node.commit_index,
            last_applied: node.last_applied,
            state_machine: node.state_machine.clone(),
        }
    }
}

pub trait StorageStrategy {
    fn load(&self, _node_id: NodeId) -> Option<StorageSnapshot> {
        None
    }

    fn save(&mut self, snapshot: StorageSnapshot);
}

#[derive(Default)]
pub struct NoopStorage;

impl StorageStrategy for NoopStorage {
    fn save(&mut self, _snapshot: StorageSnapshot) {}
}

#[derive(Default)]
pub struct InMemoryStorage {
    snapshots: HashMap<NodeId, StorageSnapshot>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, node_id: NodeId) -> Option<&StorageSnapshot> {
        self.snapshots.get(&node_id)
    }
}

impl StorageStrategy for InMemoryStorage {
    fn load(&self, node_id: NodeId) -> Option<StorageSnapshot> {
        self.snapshots.get(&node_id).cloned()
    }

    fn save(&mut self, snapshot: StorageSnapshot) {
        self.snapshots.insert(snapshot.node_id, snapshot);
    }
}
