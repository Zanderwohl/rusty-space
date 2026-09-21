//! How messages get in and out.
//!
//! A trait, because the transport is still open — QUIC for native, WebTransport or WebSocket
//! for the browser, and the serialisation format undecided besides. None of that changes the
//! filter, which is what phase 8 is actually about, so none of it is here yet.
//!
//! There is one implementation: in process, for tests. It is enough to prove the thing that
//! needs proving, because the gate is in the type the channel carries rather than in the
//! channel.

use std::collections::HashMap;

use lc_proto::{ClientId, Inbound, Outbound};

pub trait Transport {
    /// Everything said since the last call, in the order it was said.
    fn poll(&mut self) -> Vec<(ClientId, Inbound)>;

    /// Say something to one client.
    ///
    /// Takes an [`Outbound`] and nothing else, so the only sightings that can reach it are the
    /// ones [`lc_proto::Cleared::clear`] made. Implementations must not grow a second method.
    fn send(&mut self, to: ClientId, message: Outbound);
}

/// Both ends in one process.
#[derive(Default)]
pub struct Loopback {
    inbox: Vec<(ClientId, Inbound)>,
    outbox: HashMap<ClientId, Vec<Outbound>>,
}

impl Loopback {
    pub fn new() -> Self {
        Self::default()
    }

    /// A client says something.
    pub fn client_says(&mut self, from: ClientId, message: Inbound) {
        self.inbox.push((from, message));
    }

    /// Everything a client has been told, and clear it.
    pub fn take(&mut self, client: ClientId) -> Vec<Outbound> {
        self.outbox.remove(&client).unwrap_or_default()
    }

    /// Everything a client has been told, without clearing it.
    pub fn peek(&self, client: ClientId) -> &[Outbound] {
        self.outbox.get(&client).map(Vec::as_slice).unwrap_or(&[])
    }
}

impl Transport for Loopback {
    fn poll(&mut self) -> Vec<(ClientId, Inbound)> {
        std::mem::take(&mut self.inbox)
    }

    fn send(&mut self, to: ClientId, message: Outbound) {
        self.outbox.entry(to).or_default().push(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_proto::PROTOCOL_VERSION;

    #[test]
    fn what_goes_in_comes_out_once() {
        let mut wire = Loopback::new();
        wire.client_says(
            ClientId(1),
            Inbound::Hello {
                protocol: PROTOCOL_VERSION,
                ticket: String::new(),
            },
        );
        assert_eq!(wire.poll().len(), 1);
        assert!(wire.poll().is_empty(), "a message was delivered twice");

        wire.send(ClientId(1), Outbound::WrongProtocol { server: 7 });
        assert_eq!(wire.peek(ClientId(1)).len(), 1);
        assert_eq!(wire.take(ClientId(1)).len(), 1);
        assert!(wire.take(ClientId(1)).is_empty());
        assert!(
            wire.take(ClientId(2)).is_empty(),
            "a client nobody wrote to has nothing"
        );
    }
}
