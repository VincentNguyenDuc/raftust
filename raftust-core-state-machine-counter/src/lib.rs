use raftust_core::StateMachineStrategy;

#[derive(Debug, Clone, Default)]
pub struct CounterStateMachine {
    value: i64,
}

impl CounterStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn value(&self) -> i64 {
        self.value
    }

    fn parse_command(raw: &str) -> Option<CounterCommand> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        if trimmed.eq_ignore_ascii_case("inc") {
            return Some(CounterCommand::Add(1));
        }
        if trimmed.eq_ignore_ascii_case("dec") {
            return Some(CounterCommand::Add(-1));
        }
        if trimmed.eq_ignore_ascii_case("reset") {
            return Some(CounterCommand::Reset);
        }

        if let Some(delta) = trimmed.strip_prefix("add ") {
            let amount = delta.trim().parse::<i64>().ok()?;
            return Some(CounterCommand::Add(amount));
        }

        if let Some(value) = trimmed.strip_prefix("set ") {
            let value = value.trim().parse::<i64>().ok()?;
            return Some(CounterCommand::Set(value));
        }

        None
    }
}

impl StateMachineStrategy for CounterStateMachine {
    fn apply(&mut self, raw: &str) {
        if let Some(command) = Self::parse_command(raw) {
            match command {
                CounterCommand::Add(delta) => {
                    self.value = self.value.saturating_add(delta);
                }
                CounterCommand::Set(value) => {
                    self.value = value;
                }
                CounterCommand::Reset => {
                    self.value = 0;
                }
            }
        }
    }

    fn describe(&self) -> String {
        format!("counter={}", self.value)
    }

    fn snapshot(&self) -> Vec<u8> {
        self.value.to_string().into_bytes()
    }

    fn restore(&mut self, snapshot: &[u8]) -> Result<(), String> {
        if snapshot.is_empty() {
            self.value = 0;
            return Ok(());
        }

        let raw = std::str::from_utf8(snapshot).map_err(|err| format!("utf8 snapshot: {}", err))?;
        self.value = raw
            .parse::<i64>()
            .map_err(|err| format!("parse counter snapshot: {}", err))?;
        Ok(())
    }
}

enum CounterCommand {
    Add(i64),
    Set(i64),
    Reset,
}

#[cfg(test)]
mod tests {
    use super::CounterStateMachine;
    use raftust_core::StateMachineStrategy;

    #[test]
    fn applies_basic_counter_commands() {
        let mut sm = CounterStateMachine::new();

        sm.apply("inc");
        sm.apply("add 4");
        sm.apply("dec");

        assert_eq!(sm.value(), 4);
        assert_eq!(sm.describe(), "counter=4");
    }

    #[test]
    fn supports_set_and_reset() {
        let mut sm = CounterStateMachine::new();

        sm.apply("set 10");
        sm.apply("reset");

        assert_eq!(sm.value(), 0);
    }

    #[test]
    fn ignores_invalid_commands() {
        let mut sm = CounterStateMachine::new();

        sm.apply("add nope");
        sm.apply("something else");

        assert_eq!(sm.value(), 0);
    }

    #[test]
    fn handles_whitespace_and_case_for_keywords() {
        let mut sm = CounterStateMachine::new();

        sm.apply("  INC  ");
        sm.apply("  dec");
        sm.apply("  reset   ");

        assert_eq!(sm.value(), 0);
        assert_eq!(sm.describe(), "counter=0");
    }

    #[test]
    fn supports_negative_add_amounts() {
        let mut sm = CounterStateMachine::new();

        sm.apply("set 10");
        sm.apply("add -3");

        assert_eq!(sm.value(), 7);
    }

    #[test]
    fn add_saturates_at_i64_bounds() {
        let mut sm = CounterStateMachine::new();

        sm.apply(&format!("set {}", i64::MAX));
        sm.apply("inc");
        assert_eq!(sm.value(), i64::MAX);

        sm.apply(&format!("set {}", i64::MIN));
        sm.apply("dec");
        assert_eq!(sm.value(), i64::MIN);
    }

    #[test]
    fn snapshot_roundtrip_restores_counter_value() {
        let mut original = CounterStateMachine::new();
        original.apply("set 42");

        let bytes = original.snapshot();

        let mut restored = CounterStateMachine::new();
        restored
            .restore(&bytes)
            .expect("restore should parse valid counter snapshot");

        assert_eq!(restored.value(), 42);
    }

    #[test]
    fn restore_rejects_invalid_snapshot_bytes() {
        let mut sm = CounterStateMachine::new();
        let err = sm
            .restore(b"not-a-number")
            .expect_err("invalid bytes must fail");
        assert!(err.contains("parse counter snapshot"));
    }
}
