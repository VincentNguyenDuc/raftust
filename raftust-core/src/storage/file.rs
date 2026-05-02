use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::storage::{StorageSnapshot, StorageStrategy};
use crate::{LogEntry, NodeId};

pub struct FileStorage {
    directory: PathBuf,
}

impl FileStorage {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn directory(&self) -> &PathBuf {
        &self.directory
    }

    fn snapshot_path(&self, node_id: NodeId) -> PathBuf {
        self.directory.join(format!("node-{}.json", node_id))
    }

    fn temp_snapshot_path(&self, node_id: NodeId) -> PathBuf {
        self.directory.join(format!("node-{}.json.tmp", node_id))
    }

    fn load_snapshot_file(&self, node_id: NodeId) -> Option<StorageSnapshot> {
        let path = self.snapshot_path(node_id);
        let raw = fs::read_to_string(path).ok()?;
        let persisted = serde_json::from_str::<PersistedSnapshot>(&raw).ok()?;
        Some(persisted.into())
    }

    fn save_snapshot_file(&self, snapshot: StorageSnapshot) -> Result<(), String> {
        fs::create_dir_all(&self.directory).map_err(|err| err.to_string())?;

        let node_id = snapshot.node_id;
        let persisted: PersistedSnapshot = snapshot.into();
        let raw = serde_json::to_string(&persisted).map_err(|err| err.to_string())?;

        let tmp_path = self.temp_snapshot_path(node_id);
        let final_path = self.snapshot_path(node_id);

        let mut file = fs::File::create(&tmp_path).map_err(|err| err.to_string())?;
        file.write_all(raw.as_bytes())
            .map_err(|err| err.to_string())?;
        file.sync_all().map_err(|err| err.to_string())?;

        fs::rename(tmp_path, final_path).map_err(|err| err.to_string())?;
        Ok(())
    }
}

impl StorageStrategy for FileStorage {
    fn load(&self, node_id: NodeId) -> Option<StorageSnapshot> {
        self.load_snapshot_file(node_id)
    }

    fn save(&mut self, snapshot: StorageSnapshot) {
        if let Err(err) = self.save_snapshot_file(snapshot) {
            eprintln!("file storage save error: {err}");
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedLogEntry {
    term: u64,
    command: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedSnapshot {
    node_id: NodeId,
    current_term: u64,
    voted_for: Option<NodeId>,
    log: Vec<PersistedLogEntry>,
    commit_index: usize,
}

impl From<StorageSnapshot> for PersistedSnapshot {
    fn from(snapshot: StorageSnapshot) -> Self {
        Self {
            node_id: snapshot.node_id,
            current_term: snapshot.current_term,
            voted_for: snapshot.voted_for,
            log: snapshot
                .log
                .into_iter()
                .map(|entry| PersistedLogEntry {
                    term: entry.term,
                    command: entry.command,
                })
                .collect(),
            commit_index: snapshot.commit_index,
        }
    }
}

impl From<PersistedSnapshot> for StorageSnapshot {
    fn from(snapshot: PersistedSnapshot) -> Self {
        Self {
            node_id: snapshot.node_id,
            current_term: snapshot.current_term,
            voted_for: snapshot.voted_for,
            log: snapshot
                .log
                .into_iter()
                .map(|entry| LogEntry {
                    term: entry.term,
                    command: entry.command,
                })
                .collect(),
            commit_index: snapshot.commit_index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FileStorage;
    use crate::LogEntry;
    use crate::storage::{StorageSnapshot, StorageStrategy};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("raftust-file-storage-test-{}", nanos))
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = temp_dir();
        let mut storage = FileStorage::new(&dir);

        let snapshot = StorageSnapshot {
            node_id: 1,
            current_term: 3,
            voted_for: Some(2),
            log: vec![
                LogEntry {
                    term: 1,
                    command: "set color blue".to_string(),
                },
                LogEntry {
                    term: 2,
                    command: "set size large".to_string(),
                },
            ],
            commit_index: 2,
        };

        storage.save(snapshot.clone());

        let loaded = storage.load(1).expect("snapshot should exist");
        assert_eq!(loaded.node_id, snapshot.node_id);
        assert_eq!(loaded.current_term, snapshot.current_term);
        assert_eq!(loaded.voted_for, snapshot.voted_for);
        assert_eq!(loaded.commit_index, snapshot.commit_index);
        assert_eq!(loaded.log.len(), 2);
        assert_eq!(loaded.log[1].command, "set size large");

        fs::remove_dir_all(dir).expect("cleanup should remove temp dir");
    }

    #[test]
    fn load_returns_none_when_file_missing() {
        let dir = temp_dir();
        let storage = FileStorage::new(&dir);

        let loaded = storage.load(42);
        assert!(loaded.is_none());
    }
}
