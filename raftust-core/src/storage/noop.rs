use crate::storage::{StorageSnapshot, StorageStrategy};

#[derive(Default)]
pub struct NoopStorage;

impl StorageStrategy for NoopStorage {
    fn save(&mut self, _snapshot: StorageSnapshot) {}
}
