pub trait StateMachineStrategy {
    fn apply(&mut self, raw: &str);

    fn snapshot(&self) -> Vec<u8> {
        Vec::new()
    }

    fn restore(&mut self, _snapshot: &[u8]) -> Result<(), String> {
        Ok(())
    }

    fn describe(&self) -> String {
        "<state-machine>".to_string()
    }
}
