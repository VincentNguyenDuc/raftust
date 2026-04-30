use std::path::PathBuf;

use crate::NodeId;
use crate::storage::{StorageSnapshot, StorageStrategy};

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
}

impl StorageStrategy for FileStorage {
    fn load(&self, _node_id: NodeId) -> Option<StorageSnapshot> {
        None
    }

    fn save(&mut self, _snapshot: StorageSnapshot) {
        // Scaffold: persistence implementation to be added.
    }
}
