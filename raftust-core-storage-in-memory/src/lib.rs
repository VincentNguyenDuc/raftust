use std::collections::HashMap;

use log::debug;
use raftust_core::{NodeId, StorageSnapshot, StorageStrategy};

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
        let loaded = self.snapshots.get(&node_id).cloned();
        debug!(
            "event=storage_memory_load node_id={} hit={} stored_nodes={}",
            node_id,
            loaded.is_some(),
            self.snapshots.len()
        );
        loaded
    }

    fn save(&mut self, snapshot: StorageSnapshot) {
        debug!(
            "event=storage_memory_save node_id={} commit_index={} log_len={} snapshot_index={} snapshot_term={}",
            snapshot.node_id,
            snapshot.commit_index,
            snapshot.log.len(),
            snapshot.last_included_index,
            snapshot.last_included_term
        );
        self.snapshots.insert(snapshot.node_id, snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::InMemoryStorage;
    use raftust_core::{LogEntry, StorageSnapshot, StorageStrategy};

    fn snapshot(
        node_id: u64,
        current_term: u64,
        command: &str,
        commit_index: usize,
    ) -> StorageSnapshot {
        StorageSnapshot {
            node_id,
            current_term,
            voted_for: None,
            log: vec![LogEntry {
                term: current_term,
                command: command.to_string(),
            }],
            commit_index,
            last_included_index: 0,
            last_included_term: 0,
            state_machine_snapshot: Vec::new(),
        }
    }

    #[test]
    fn load_returns_none_for_unknown_node() {
        let storage = InMemoryStorage::new();
        assert!(storage.load(42).is_none());
    }

    #[test]
    fn save_then_load_round_trips_snapshot() {
        let mut storage = InMemoryStorage::new();
        let original = snapshot(1, 3, "set key value", 1);

        storage.save(original.clone());
        let loaded = storage.load(1).expect("snapshot should exist");

        assert_eq!(loaded.node_id, original.node_id);
        assert_eq!(loaded.current_term, original.current_term);
        assert_eq!(loaded.voted_for, original.voted_for);
        assert_eq!(loaded.log, original.log);
        assert_eq!(loaded.commit_index, original.commit_index);
        assert_eq!(loaded.last_included_index, original.last_included_index);
        assert_eq!(loaded.last_included_term, original.last_included_term);
        assert_eq!(
            loaded.state_machine_snapshot,
            original.state_machine_snapshot
        );
    }

    #[test]
    fn save_overwrites_existing_snapshot_for_same_node() {
        let mut storage = InMemoryStorage::new();

        storage.save(snapshot(7, 1, "set a 1", 0));
        storage.save(snapshot(7, 2, "set a 2", 1));

        let loaded = storage.load(7).expect("snapshot should exist");
        assert_eq!(loaded.current_term, 2);
        assert_eq!(loaded.commit_index, 1);
        assert_eq!(loaded.log[0].command, "set a 2");
    }
}
