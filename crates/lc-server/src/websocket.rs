//! A WebSocket transport.
//!
//! Chosen because it is the only thing that reaches every target today — desktop and browser,
//! with no TLS ceremony in development — and because almost all of this game's traffic wants
//! reliable ordered delivery anyway. The one channel that would want datagrams is the player's
//! own ship state, and the client predicts that locally.
//!
//! What it costs is head-of-line blocking: one stream, so a bulk transfer would stall events.
//! Bulk is already "out of band" in `lightcone/docs/08-networking.md` and belongs on its own
//! connection when it exists.
//!
//! The IO is async and the tick is not. Reader and writer tasks own the sockets; the tick only
//! ever touches channel ends, so it never awaits on a peer.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lc_proto::{ClientId, Inbound, Outbound};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;

use crate::transport::Transport;

type Outboxes = Arc<Mutex<HashMap<ClientId, UnboundedSender<Outbound>>>>;

/// A listening server. Dropping it stops accepting; open connections close with their tasks.
pub struct WebSocketServer {
    /// Where the socket is actually listening, which matters when the port was left to the OS.
    pub local_addr: std::net::SocketAddr,
    inbox: UnboundedReceiver<(ClientId, Inbound)>,
    joined: UnboundedReceiver<ClientId>,
    parted: UnboundedReceiver<ClientId>,
    outboxes: Outboxes,
}

impl WebSocketServer {
    /// Listen. `addr` may end in `:0` to let the operating system choose a port.
    pub async fn bind(addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let (to_inbox, inbox) = unbounded_channel();
        let (to_joined, joined) = unbounded_channel();
        let (to_parted, parted) = unbounded_channel();
        let outboxes: Outboxes = Arc::new(Mutex::new(HashMap::new()));

        let accepting = outboxes.clone();
        tokio::spawn(async move {
            // Identifiers come from here because who is connected is the transport's fact.
            let next = AtomicU64::new(1);
            while let Ok((stream, _)) = listener.accept().await {
                let id = ClientId(next.fetch_add(1, Ordering::Relaxed));
                let (to_inbox, to_joined, to_parted) =
                    (to_inbox.clone(), to_joined.clone(), to_parted.clone());
                let outboxes = accepting.clone();
                tokio::spawn(async move {
                    serve(id, stream, outboxes, to_inbox, to_joined, to_parted).await;
                });
            }
        });

        Ok(Self { local_addr, inbox, joined, parted, outboxes })
    }

    /// Connections opened since the last call. The caller admits them to the world.
    pub fn accepted(&mut self) -> Vec<ClientId> {
        drain(&mut self.joined)
    }

    /// Connections closed since the last call.
    pub fn departed(&mut self) -> Vec<ClientId> {
        drain(&mut self.parted)
    }
}

fn drain<T>(channel: &mut UnboundedReceiver<T>) -> Vec<T> {
    let mut out = Vec::new();
    while let Ok(item) = channel.try_recv() {
        out.push(item);
    }
    out
}

async fn serve(
    id: ClientId,
    stream: TcpStream,
    outboxes: Outboxes,
    to_inbox: UnboundedSender<(ClientId, Inbound)>,
    to_joined: UnboundedSender<ClientId>,
    to_parted: UnboundedSender<ClientId>,
) {
    use futures_util::{SinkExt, StreamExt};

    let Ok(socket) = tokio_tungstenite::accept_async(stream).await else { return };
    let (mut writer, mut reader) = socket.split();
    let (to_client, mut outbox) = unbounded_channel::<Outbound>();
    outboxes.lock().await.insert(id, to_client);
    let _ = to_joined.send(id);

    let writing = tokio::spawn(async move {
        while let Some(message) = outbox.recv().await {
            if writer.send(Message::Binary(lc_proto::encode(&message))).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(frame)) = reader.next().await {
        let bytes = match frame {
            Message::Binary(bytes) => bytes,
            Message::Close(_) => break,
            // Text, ping and pong are not this protocol. Ignored rather than fatal: a browser
            // or a proxy may send them on its own account.
            _ => continue,
        };
        match lc_proto::decode::<Inbound>(&bytes) {
            Ok(message) => {
                if to_inbox.send((id, message)).is_err() {
                    break;
                }
            }
            // Undecodable means the peer is not speaking this version, whatever it claimed.
            // There is nothing to say back that it could read.
            Err(_) => break,
        }
    }

    outboxes.lock().await.remove(&id);
    writing.abort();
    let _ = to_parted.send(id);
}

impl Transport for WebSocketServer {
    fn poll(&mut self) -> Vec<(ClientId, Inbound)> {
        drain(&mut self.inbox)
    }

    fn send(&mut self, to: ClientId, message: Outbound) {
        // `try_lock` rather than blocking: the tick must never wait on IO, and the only thing
        // that holds this lock is a connection opening or closing. A send that loses the race
        // is dropped, which is why the map is touched and not the socket -- an unbounded
        // channel behind it means the writer task never makes the tick wait either.
        if let Ok(map) = self.outboxes.try_lock()
            && let Some(channel) = map.get(&to)
        {
            let _ = channel.send(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use lc_proto::{Intent, Order, PROTOCOL_VERSION, ShipId};

    use super::*;
    use crate::journal::Memory;
    use crate::server::Server;
    use crate::world::Path;

    /// Wait for something to turn up, rather than sleeping a fixed time and hoping.
    async fn until<T>(mut f: impl FnMut() -> Vec<T>) -> Vec<T> {
        for _ in 0..200 {
            let got = f();
            if !got.is_empty() {
                return got;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        Vec::new()
    }

    /// A real socket, a real handshake, and the protocol's own bytes over it.
    #[tokio::test]
    async fn a_client_connects_over_a_socket_and_is_welcomed() {
        let mut wire = WebSocketServer::bind("127.0.0.1:0").await.expect("a port");
        let url = format!("ws://{}", wire.local_addr);
        let (mut client, _) = tokio_tungstenite::connect_async(&url).await.expect("connected");

        let joined = until(|| wire.accepted()).await;
        assert_eq!(joined.len(), 1, "the connection was never seen");
        let id = joined[0];

        let mut server = Server::new(Memory::default(), 0, 1);
        server.admit(id, ShipId(1), Path::still(glam::DVec3::ZERO), 0.0);

        client
            .send(Message::Binary(
                lc_proto::encode(&Inbound::Hello { protocol: PROTOCOL_VERSION }),
            ))
            .await
            .unwrap();

        // Tick until the hello has come through and been answered.
        for _ in 0..200 {
            server.tick(&mut wire).await.unwrap();
            if let Ok(Some(Ok(frame))) = tokio::time::timeout(
                std::time::Duration::from_millis(5),
                client.next(),
            )
            .await
            {
                let Message::Binary(bytes) = frame else { continue };
                let message: Outbound = lc_proto::decode(&bytes).expect("it decodes");
                assert!(
                    matches!(message, Outbound::Welcome { client_id, ship_id, .. }
                        if client_id == id && ship_id == ShipId(1)),
                    "{message:?}",
                );
                return;
            }
        }
        panic!("no welcome came back over the socket");
    }

    /// The whole stack over a socket: act, and be told about it.
    #[tokio::test]
    async fn an_intent_crosses_the_wire_and_comes_back_as_a_sighting() {
        let mut wire = WebSocketServer::bind("127.0.0.1:0").await.expect("a port");
        let url = format!("ws://{}", wire.local_addr);
        let (mut client, _) = tokio_tungstenite::connect_async(&url).await.expect("connected");
        let id = until(|| wire.accepted()).await[0];

        let mut server = Server::new(Memory::default(), 0, 1);
        server.admit(id, ShipId(1), Path::still(glam::DVec3::ZERO), 0.0);

        client
            .send(Message::Binary(
                lc_proto::encode(&Inbound::Act(Intent {
                    ship_id: ShipId(1),
                    order: Order::Transmit { power_w: 1.0e9 },
                    issued_at_client_t: 0,
                })),
            ))
            .await
            .unwrap();

        for _ in 0..200 {
            server.tick(&mut wire).await.unwrap();
            if let Ok(Some(Ok(frame))) = tokio::time::timeout(
                std::time::Duration::from_millis(5),
                client.next(),
            )
            .await
            {
                let Message::Binary(bytes) = frame else { continue };
                let message: Outbound = lc_proto::decode(&bytes).expect("it decodes");
                let Outbound::Sightings(list) = message else { continue };
                assert_eq!(list.len(), 1);
                // Its own act, at its own position, so it is told at once.
                assert_eq!(list[0].get().source_id, 1);
                return;
            }
        }
        panic!("the sighting never came back");
    }

    /// A departing client is noticed, so the world can stop holding a ship for it.
    #[tokio::test]
    async fn a_closed_connection_is_reported() {
        let mut wire = WebSocketServer::bind("127.0.0.1:0").await.expect("a port");
        let url = format!("ws://{}", wire.local_addr);
        let (client, _) = tokio_tungstenite::connect_async(&url).await.expect("connected");
        let id = until(|| wire.accepted()).await[0];
        drop(client);
        assert_eq!(until(|| wire.departed()).await, vec![id]);
    }

    /// Sending to nobody is not an error. Connections close whenever they like, and a tick that
    /// panicked on one would be a tick a client could end.
    #[tokio::test]
    async fn sending_to_a_client_that_has_gone_is_harmless() {
        let mut wire = WebSocketServer::bind("127.0.0.1:0").await.expect("a port");
        wire.send(ClientId(999), Outbound::WrongProtocol { server: 1 });
    }
}
