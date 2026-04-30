pub mod network;
pub mod wire;

pub use network::NetworkTransport;
pub use wire::WireMessage;

pub trait TransportStrategy {
    fn start_listener(&mut self, addr: String) -> Result<(), String>;
    fn try_recv(&mut self) -> Option<WireMessage>;
    fn send(&mut self, msg: WireMessage);
}
