use std::collections::HashMap;

enum StateMachineCommand {
    Set { key: String, value: String },
    Delete { key: String },
}

pub fn apply_command(state: &mut HashMap<String, String>, raw: &str) {
    if let Some(cmd) = parse_command(raw) {
        match cmd {
            StateMachineCommand::Set { key, value } => {
                state.insert(key, value);
            }
            StateMachineCommand::Delete { key } => {
                state.remove(&key);
            }
        }
    }
}

fn parse_command(raw: &str) -> Option<StateMachineCommand> {
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
