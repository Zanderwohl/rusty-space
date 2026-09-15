//! The client's half of the wire.
//!
//! A trait with two implementations behind it, for the reason `lc_server::transport::Transport`
//! is one: the desktop build opens a socket itself and the browser build is handed one by the
//! page, and nothing above this should be able to tell which.
//!
//! **Poll-based, never blocking.** A frame cannot await, so everything here either has already
//! happened or has not. The native implementation owns a thread and talks to it over channels;
//! the browser's will own callbacks and do the same, which is why the interface is shaped like
//! this rather than around a future.

use lc_proto::{Inbound, Outbound};

/// Where a connection has got to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The socket is opening. Nothing may be sent yet, and messages handed over now are queued.
    Connecting,
    Open,
    /// Gone, with something to show a person. Terminal: a `Link` is not reconnected, it is
    /// replaced, so there is no state here that could be stale.
    Closed(String),
}

impl Status {
    pub fn is_open(&self) -> bool {
        matches!(self, Status::Open)
    }
}

pub trait Link {
    /// Everything the server has said since the last call, in order.
    fn poll(&mut self) -> Vec<Outbound>;

    /// Say something. Queued while [`Status::Connecting`], dropped once closed — a caller that
    /// had to check first would have to handle the race anyway, because the socket can close
    /// between the check and the send.
    fn send(&mut self, message: Inbound);

    fn status(&self) -> Status;
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::WebSocketLink;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::net::TcpStream;
    use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use lc_proto::{Inbound, Outbound};
    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket};

    use super::{Link, Status};

    /// How long the reader blocks before looking at the outgoing queue.
    ///
    /// The whole cost of not having an async runtime here: a message waits up to this long to
    /// be written. Well under a tick, so nothing observes it.
    const POLL_INTERVAL: Duration = Duration::from_millis(10);

    /// A WebSocket on a thread of its own.
    pub struct WebSocketLink {
        outgoing: Sender<Inbound>,
        incoming: Receiver<Outbound>,
        status: Arc<Mutex<Status>>,
    }

    impl WebSocketLink {
        /// Start connecting. Returns immediately — the socket opens on the thread, and the
        /// caller watches [`Link::status`].
        pub fn connect(url: &str) -> Self {
            let (to_server, from_app) = channel::<Inbound>();
            let (to_app, from_server) = channel::<Outbound>();
            let status = Arc::new(Mutex::new(Status::Connecting));

            let url = url.to_owned();
            let thread_status = status.clone();
            std::thread::Builder::new()
                .name("lc-link".into())
                .spawn(move || run(&url, &from_app, &to_app, &thread_status))
                .expect("a thread");

            Self {
                outgoing: to_server,
                incoming: from_server,
                status,
            }
        }
    }

    fn run(
        url: &str,
        from_app: &Receiver<Inbound>,
        to_app: &Sender<Outbound>,
        status: &Arc<Mutex<Status>>,
    ) {
        let mut socket = match tungstenite::connect(url) {
            Ok((socket, _)) => socket,
            Err(why) => {
                *status.lock().unwrap() = Status::Closed(why.to_string());
                return;
            }
        };
        if let Err(why) = set_read_timeout(&mut socket, POLL_INTERVAL) {
            // Without it the read below blocks forever and nothing is ever written.
            *status.lock().unwrap() = Status::Closed(why);
            return;
        }
        *status.lock().unwrap() = Status::Open;

        let closed = loop {
            // Writes first, so a message handed over while connecting goes out on the first
            // pass rather than after a timeout.
            match drain_outgoing(&mut socket, from_app) {
                Draining::Empty => {}
                Draining::Failed(why) => break why,
                // The app dropped its end: nobody is listening and nobody will send.
                Draining::AppGone => return,
            }

            match socket.read() {
                Ok(Message::Binary(bytes)) => match lc_proto::decode::<Outbound>(&bytes) {
                    Ok(message) => {
                        if to_app.send(message).is_err() {
                            return;
                        }
                    }
                    // The peer is not speaking this version, whatever it claimed. There is
                    // nothing to say back that it could read.
                    Err(why) => break format!("undecodable message: {why}"),
                },
                Ok(Message::Close(_)) => break "the server closed the connection".into(),
                // Text, ping and pong are not this protocol. Tungstenite answers pings itself.
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    // The timeout, which is the only way out of a blocking read and is how the
                    // outgoing queue gets looked at at all.
                    continue;
                }
                Err(why) => break why.to_string(),
            }
        };
        *status.lock().unwrap() = Status::Closed(closed);
    }

    enum Draining {
        Empty,
        Failed(String),
        AppGone,
    }

    fn drain_outgoing(
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        from_app: &Receiver<Inbound>,
    ) -> Draining {
        loop {
            match from_app.try_recv() {
                Ok(message) => {
                    if let Err(why) = socket.send(Message::Binary(lc_proto::encode(&message))) {
                        return Draining::Failed(why.to_string());
                    }
                }
                Err(TryRecvError::Empty) => return Draining::Empty,
                Err(TryRecvError::Disconnected) => return Draining::AppGone,
            }
        }
    }

    /// Reach the `TcpStream` under whatever the scheme wrapped it in.
    fn set_read_timeout(
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        how_long: Duration,
    ) -> Result<(), String> {
        let stream: &TcpStream = match socket.get_ref() {
            MaybeTlsStream::Plain(stream) => stream,
            MaybeTlsStream::Rustls(stream) => stream.get_ref(),
            other => return Err(format!("unsupported stream: {other:?}")),
        };
        stream
            .set_read_timeout(Some(how_long))
            .map_err(|e| e.to_string())
    }

    impl Link for WebSocketLink {
        fn poll(&mut self) -> Vec<Outbound> {
            self.incoming.try_iter().collect()
        }

        fn send(&mut self, message: Inbound) {
            // A closed socket is a dropped receiver, which is the error here. Nothing to do
            // about it: `status` is what a caller watches.
            let _ = self.outgoing.send(message);
        }

        fn status(&self) -> Status {
            self.status.lock().unwrap().clone()
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::BrowserLink;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::sync::{Arc, Mutex};

    use lc_proto::{Inbound, Outbound};
    use send_wrapper::SendWrapper;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

    use super::{Link, Status};

    /// What the callbacks write and the frame reads.
    ///
    /// `Arc<Mutex<_>>` rather than the `Rc<RefCell<_>>` a browser-only type would normally use,
    /// because the struct holding it has to be `Send` to live in a Bevy resource. On a
    /// single-threaded target the lock is never contended and costs nothing.
    #[derive(Default)]
    struct Shared {
        incoming: Vec<Outbound>,
        /// Set once, by whichever of close, error or a bad frame happens first.
        closed: Option<String>,
    }

    /// The socket, and the closures that have to outlive the call that registered them.
    ///
    /// Dropping a `Closure` unregisters it, so they are kept rather than `forget()`-ed — a
    /// forgotten closure is a leak of one per connection, which a reconnecting client repeats.
    struct Inner {
        socket: WebSocket,
        _on_message: Closure<dyn FnMut(MessageEvent)>,
        _on_close: Closure<dyn FnMut(CloseEvent)>,
        _on_error: Closure<dyn FnMut(web_sys::Event)>,
    }

    /// A WebSocket the browser owns.
    ///
    /// No thread, because there is none to have: the browser calls back into this on the same
    /// task the frame runs on. That is why [`Link`] is shaped around polling rather than around
    /// a future — the two implementations want the same interface for opposite reasons.
    pub struct BrowserLink {
        /// `SendWrapper` because `WebSocket` and `Closure` are `!Send` and a Bevy resource must
        /// be `Send`. Sound here for the reason it is sound anywhere: this target has one
        /// thread, so the check it makes on every access can never fail.
        inner: SendWrapper<Inner>,
        shared: Arc<Mutex<Shared>>,
        /// Handed over before the socket opened. `WebSocket.send` throws until then, so these
        /// wait for the first poll that finds it open.
        queued: Vec<Inbound>,
    }

    impl BrowserLink {
        pub fn connect(url: &str) -> Result<Self, String> {
            let socket = WebSocket::new(url).map_err(|why| describe(&why))?;
            // Without this the browser delivers a `Blob`, which can only be read
            // asynchronously — and a frame has nothing to await on.
            socket.set_binary_type(BinaryType::Arraybuffer);
            let shared = Arc::new(Mutex::new(Shared::default()));

            let on_message = {
                let shared = shared.clone();
                Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                    let Ok(buffer) = event.data().dyn_into::<js_sys::ArrayBuffer>() else {
                        // Text, or something else this protocol does not speak. Ignored rather
                        // than fatal: a proxy may send its own.
                        return;
                    };
                    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                    let mut shared = shared.lock().unwrap();
                    match lc_proto::decode::<Outbound>(&bytes) {
                        Ok(message) => shared.incoming.push(message),
                        // The peer is not speaking this version, whatever it claimed.
                        Err(why) => shared.closed = Some(format!("undecodable message: {why}")),
                    }
                })
            };
            socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

            let on_close = {
                let shared = shared.clone();
                Closure::<dyn FnMut(CloseEvent)>::new(move |event: CloseEvent| {
                    let said = event.reason();
                    let why = if said.is_empty() {
                        format!("the server closed the connection ({})", event.code())
                    } else {
                        said
                    };
                    let mut shared = shared.lock().unwrap();
                    shared.closed.get_or_insert(why);
                })
            };
            socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

            let on_error = {
                let shared = shared.clone();
                Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                    // A browser deliberately says nothing about why a socket failed — telling
                    // a page would let it probe the network it is on. There is nothing more to
                    // report than that it did.
                    let mut shared = shared.lock().unwrap();
                    shared.closed.get_or_insert_with(|| "the connection failed".to_string());
                })
            };
            socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

            Ok(Self {
                inner: SendWrapper::new(Inner {
                    socket,
                    _on_message: on_message,
                    _on_close: on_close,
                    _on_error: on_error,
                }),
                shared,
                queued: Vec::new(),
            })
        }

        fn is_open(&self) -> bool {
            self.inner.socket.ready_state() == WebSocket::OPEN
        }
    }

    fn describe(why: &wasm_bindgen::JsValue) -> String {
        why.as_string().unwrap_or_else(|| "the socket could not be opened".to_string())
    }

    impl Link for BrowserLink {
        fn poll(&mut self) -> Vec<Outbound> {
            // Anything handed over while the socket was still opening goes out now. Before the
            // first open there is nowhere to put it but here.
            if self.is_open() && !self.queued.is_empty() {
                for message in std::mem::take(&mut self.queued) {
                    let _ = self.inner.socket.send_with_u8_array(&lc_proto::encode(&message));
                }
            }
            std::mem::take(&mut self.shared.lock().unwrap().incoming)
        }

        fn send(&mut self, message: Inbound) {
            if self.is_open() {
                let _ = self.inner.socket.send_with_u8_array(&lc_proto::encode(&message));
            } else {
                self.queued.push(message);
            }
        }

        fn status(&self) -> Status {
            // The recorded reason wins over the ready state, which only ever says *that* it
            // closed.
            if let Some(why) = &self.shared.lock().unwrap().closed {
                return Status::Closed(why.clone());
            }
            match self.inner.socket.ready_state() {
                WebSocket::CONNECTING => Status::Connecting,
                WebSocket::OPEN => Status::Open,
                _ => Status::Closed("the connection closed".to_string()),
            }
        }
    }
}

/// A link with no socket under it, for tests and for the offline build.
#[derive(Default)]
pub struct Offline {
    /// What the "server" will say next, oldest first.
    pub queued: Vec<Outbound>,
    /// Everything the client has said.
    pub sent: Vec<Inbound>,
    pub status: Option<Status>,
}

impl Offline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue something for the client to receive on its next poll.
    pub fn will_say(&mut self, message: Outbound) {
        self.queued.push(message);
    }
}

impl Link for Offline {
    fn poll(&mut self) -> Vec<Outbound> {
        std::mem::take(&mut self.queued)
    }

    fn send(&mut self, message: Inbound) {
        self.sent.push(message);
    }

    fn status(&self) -> Status {
        self.status.clone().unwrap_or(Status::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_proto::PROTOCOL_VERSION;

    #[test]
    fn what_is_queued_is_delivered_once() {
        let mut link = Offline::new();
        link.will_say(Outbound::WrongProtocol { server: 7 });
        assert_eq!(link.poll().len(), 1);
        assert!(link.poll().is_empty(), "a message was delivered twice");
    }

    #[test]
    fn what_is_sent_is_remembered_in_order() {
        let mut link = Offline::new();
        link.send(Inbound::Hello {
            protocol: PROTOCOL_VERSION,
            ticket: "a".into(),
        });
        link.send(Inbound::ResumeFrom { arrive_t: 5 });
        assert!(matches!(
            link.sent.as_slice(),
            [Inbound::Hello { .. }, Inbound::ResumeFrom { .. }]
        ));
    }

    #[test]
    fn a_closed_link_is_not_open() {
        assert!(Status::Open.is_open());
        assert!(!Status::Connecting.is_open());
        assert!(!Status::Closed("gone".into()).is_open());
    }
}
