use std::collections::HashMap;

use crate::NodeId;
use crate::storage::{StorageSnapshot, StorageStrategy};

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
