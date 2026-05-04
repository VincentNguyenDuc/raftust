use std::collections::HashMap;

use raftust_core::StateMachineStrategy;

#[derive(Debug, Default, Clone)]
pub struct KeyValueStateMachine {
    state: HashMap<String, String>,
}

impl KeyValueStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    fn parse_command(&self, raw: &str) -> Option<StateMachineCommand> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        if let Some(rest) = trimmed.strip_prefix("set ") {
            let (key, value) = rest.split_once(' ')?;
            if key.is_empty() {
                return None;
            }
            return Some(StateMachineCommand::Set {
                key: key.to_string(),
                value: value.to_string(),
            });
        }

        if let Some(key) = trimmed.strip_prefix("del ") {
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            return Some(StateMachineCommand::Delete {
                key: key.to_string(),
            });
        }

        None
    }
}

impl StateMachineStrategy for KeyValueStateMachine {
    fn apply(&mut self, raw: &str) {
        if let Some(cmd) = self.parse_command(raw) {
            match cmd {
                StateMachineCommand::Set { key, value } => {
                    self.state.insert(key, value);
                }
                StateMachineCommand::Delete { key } => {
                    self.state.remove(&key);
                }
            }
        }
    }

    fn describe(&self) -> String {
        format!("{:?}", self.state)
    }

    fn snapshot(&self) -> Vec<u8> {
        serde_json::to_vec(&self.state).unwrap_or_default()
    }

    fn restore(&mut self, snapshot: &[u8]) -> Result<(), String> {
        if snapshot.is_empty() {
            self.state.clear();
            return Ok(());
        }

        self.state =
            serde_json::from_slice(snapshot).map_err(|err| format!("restore snapshot: {}", err))?;
        Ok(())
    }
}

enum StateMachineCommand {
    Set { key: String, value: String },
    Delete { key: String },
}

#[cfg(test)]
mod tests {
    use super::KeyValueStateMachine;
    use raftust_core::StateMachineStrategy;

    #[test]
    fn applies_set_and_delete_commands() {
        let mut sm = KeyValueStateMachine::new();

        sm.apply("set color blue");
        sm.apply("set size large");
        sm.apply("del color");

        assert_eq!(sm.describe(), "{\"size\": \"large\"}");
    }

    #[test]
    fn set_supports_values_with_spaces() {
        let mut sm = KeyValueStateMachine::new();

        sm.apply("set note hello world from raft");

        assert!(
            sm.describe()
                .contains("\"note\": \"hello world from raft\"")
        );
    }

    #[test]
    fn invalid_commands_do_not_mutate_state() {
        let mut sm = KeyValueStateMachine::new();

        sm.apply("set");
        sm.apply("set    ");
        sm.apply("del");
        sm.apply("unknown command");

        assert_eq!(sm.describe(), "{}");
    }

    #[test]
    fn delete_missing_key_is_a_noop() {
        let mut sm = KeyValueStateMachine::new();

        sm.apply("set color blue");
        sm.apply("del missing");

        assert!(sm.describe().contains("\"color\": \"blue\""));
    }

    #[test]
    fn snapshot_roundtrip_restores_state() {
        let mut original = KeyValueStateMachine::new();
        original.apply("set color blue");
        original.apply("set size large");

        let bytes = original.snapshot();

        let mut restored = KeyValueStateMachine::new();
        restored
            .restore(&bytes)
            .expect("restore should succeed for valid snapshot bytes");

        let restored_desc = restored.describe();
        assert!(restored_desc.contains("\"color\": \"blue\""));
        assert!(restored_desc.contains("\"size\": \"large\""));
    }
}
