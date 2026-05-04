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
}
