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
}
