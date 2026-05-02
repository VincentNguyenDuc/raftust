pub trait StateMachineStrategy {
    fn apply(&mut self, raw: &str);

    fn describe(&self) -> String {
        "<state-machine>".to_string()
    }
}
