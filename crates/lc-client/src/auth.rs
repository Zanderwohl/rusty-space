//! Signing in from the desktop build.
//!
//! The browser build has none of this: a page that launched the game already has a session, and
//! `/play` hands it a ticket. A desktop player has neither, so the client opens the **system
//! browser** at the broker, listens on loopback for the answer, and trades it for a device grant
//! it keeps. See `lightcone/docs/16-identity.md`.
//!
//! The system browser rather than a window in the game, because that is the only way an upstream
//! provider works at all — Google will not authenticate into an embedded view it cannot show its
//! own address bar in. The local password form is the exception, and doc 16 records why that is
//! an argument against shipping the password provider rather than for embedding the rest.
//!
//! Everything here is pure or is a socket. No Bevy, so the parts worth testing are testable.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

/// Who the broker says we are. What the client keeps and shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub account_id: String,
    pub display_name: String,
}

/// A sign-in under way: what was asked for, and what the answer must match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// Generated here, echoed by the broker, and compared on the way back. Without it an
    /// attacker can complete a sign-in *as themselves* in someone else's client, and that
    /// someone goes on playing the attacker's account.
    pub state: String,
    /// The loopback the browser will be sent back to.
    pub return_to: String,
    /// Where to send the player. Opened in the system browser.
    pub open: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackError {
    /// Not the path the sign-in was started for.
    NotOurs,
    /// The `state` did not match. Someone else's sign-in, or a forged one.
    WrongState,
    /// No code in it.
    NoCode,
}

/// The loopback path the broker is configured to allow. One path, any port.
pub const RETURN_PATH: &str = "/return";

/// Begin a sign-in against `broker`, answering on `port`.
///
/// `state` is supplied rather than generated here so a test can pin it; every caller in the
/// client passes a fresh nonce.
pub fn begin(broker: &str, port: u16, state: &str) -> Pending {
    let return_to = format!("http://{}:{port}{RETURN_PATH}", Ipv4Addr::LOCALHOST);
    Pending {
        state: state.to_owned(),
        open: format!(
            "{}/signin?return_to={}&state={state}",
            broker.trim_end_matches('/'),
            percent_encode(&return_to),
        ),
        return_to,
    }
}

/// Read the code out of the browser's request line.
///
/// Anything percent-encoded is refused rather than decoded. A code is base64url and a `state`
/// is a nonce by the broker's own rule, so neither can legitimately need encoding — and a
/// decoder is a thing that can be tricked into producing a `&`.
pub fn code_from(pending: &Pending, request_line: &str) -> Result<String, CallbackError> {
    let mut parts = request_line.split_whitespace();
    // The browser arrives by `GET`, so nothing else is answered. A cross-origin form can be
    // made to POST anywhere; it cannot be made to GET with a body. Requiring the method that
    // actually happens costs nothing and removes a shape.
    if parts.next() != Some("GET") {
        return Err(CallbackError::NotOurs);
    }
    let target = parts.next().ok_or(CallbackError::NotOurs)?;
    let (path, query) = target.split_once('?').ok_or(CallbackError::NoCode)?;
    if path != RETURN_PATH || target.contains('%') {
        return Err(CallbackError::NotOurs);
    }

    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("code", value)) => code = Some(value),
            Some(("state", value)) => state = Some(value),
            _ => {}
        }
    }
    // The state first: a mismatched one means this answer is not for us, and nothing else about
    // it is worth reading.
    if state != Some(pending.state.as_str()) {
        return Err(CallbackError::WrongState);
    }
    code.filter(|c| !c.is_empty()).map(str::to_owned).ok_or(CallbackError::NoCode)
}

/// What the browser is left looking at.
///
/// Plain, and it closes nothing: a page that tries to close its own tab mostly fails and looks
/// broken when it does.
pub fn landing_page(worked: bool) -> String {
    let said = if worked {
        "Signed in. You can close this tab and go back to the game."
    } else {
        "That sign-in did not complete. Go back to the game and try again."
    };
    let body = format!(
        "<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer>\
         <title>Lightcone Frontier</title><p>{said}</p>"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    )
}

/// A bound loopback socket, before anyone knows what it is listening for.
///
/// Two steps because the port comes first: the sign-in URL contains the port, so the port has
/// to exist before there is a [`Pending`] to compare an answer against.
pub struct Bound {
    listener: TcpListener,
    pub port: u16,
}

impl Bound {
    /// Bind to a port the operating system chooses, on loopback only.
    ///
    /// The port cannot be known in advance, which is why the broker allows any port for a
    /// loopback path — RFC 8252 §7.3, and safe because `127.0.0.1` is not reachable from
    /// anywhere else.
    pub fn open() -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, port })
    }

    /// Wait for the answer to `pending`, on another thread.
    ///
    /// The sign-in is **moved in**, so the comparison against `state` happens where the request
    /// is read. An earlier version kept it in a thread-local, which the spawned thread does not
    /// share — so every answer came back as somebody else's.
    pub fn listen(self, pending: Pending) -> Loopback {
        let (sender, answer) = channel();
        // One connection, then done. A listener that stayed open would be a second way into the
        // client for as long as the game ran.
        std::thread::spawn(move || {
            let _ = sender.send(accept_one(&self.listener, &pending));
        });
        Loopback { port: self.port, answer: Mutex::new(answer) }
    }
}

/// A loopback listener waiting for one answer.
#[derive(Debug)]
pub struct Loopback {
    pub port: u16,
    /// Behind a lock because this is held in a Bevy resource, which must be `Sync`, and an
    /// `mpsc::Receiver` is `Send` and not. Never contended: one reader, one message.
    answer: Mutex<Receiver<Result<String, CallbackError>>>,
}

impl Loopback {
    /// The answer, if it has arrived. Never blocks: this is polled from a frame.
    pub fn poll(&self) -> Option<Result<String, CallbackError>> {
        self.answer.lock().ok()?.try_recv().ok()
    }
}

fn accept_one(listener: &TcpListener, pending: &Pending) -> Result<String, CallbackError> {
    let Ok((mut stream, _)) = listener.accept() else { return Err(CallbackError::NotOurs) };
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return Err(CallbackError::NotOurs);
    }
    let result = code_from(pending, line.trim_end());
    let _ = stream.write_all(landing_page(result.is_ok()).as_bytes());
    let _ = stream.flush();
    result
}

/// What the client is doing about signing in.
///
/// One enum, and the modal is a rendering of it. Anything the player can see about their own
/// sign-in is one of these, so there is no second place for "are we signed in" to be answered.
#[derive(Debug, Default)]
pub enum Session {
    /// No grant, or the one we had was refused.
    #[default]
    SignedOut,
    /// The browser is open and we are listening. The player can cancel.
    Waiting {
        loopback: Loopback,
        /// Shown so the player can paste it if the browser did not open — which happens, and
        /// leaves them staring at nothing if the address is only in a log.
        url: String,
    },
    /// Talking to the broker: trading a code for a grant, or a grant for a ticket.
    Working,
    SignedIn(Identity),
    /// Something went wrong, in words the player can act on.
    Failed(String),
}

impl Session {
    pub fn identity(&self) -> Option<&Identity> {
        match self {
            Session::SignedIn(identity) => Some(identity),
            _ => None,
        }
    }

    /// Whether the player may start observing.
    pub fn is_ready(&self) -> bool {
        matches!(self, Session::SignedIn(_))
    }

    /// Whether a modal should be in the way.
    ///
    /// Not while signed in, and not while merely signed out — pressing Observe is what asks.
    /// This is about whether the sign-in *itself* is on screen.
    pub fn is_busy(&self) -> bool {
        matches!(self, Session::Waiting { .. } | Session::Working)
    }
}

/// What a broker call came back with.
#[derive(Debug)]
pub enum BrokerError {
    /// The grant or code was refused. Recoverable by signing in again, and the only one worth
    /// clearing the vault for.
    Refused,
    /// This deployment has no password provider. Not a refusal: the form should go away rather
    /// than be tried again.
    NoPasswordProvider,
    AlreadyRegistered,
    WeakPassword,
    TooManyAttempts,
    /// Anything else: no network, a 500, a body that would not parse.
    Unreachable(String),
}

impl std::fmt::Display for BrokerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrokerError::Refused => f.write_str("that sign-in is no longer valid"),
            BrokerError::NoPasswordProvider => {
                f.write_str("this server has no password sign-in; use a browser")
            }
            BrokerError::AlreadyRegistered => f.write_str("that address already has an account"),
            BrokerError::WeakPassword => f.write_str("that password is too short"),
            BrokerError::TooManyAttempts => {
                f.write_str("too many attempts; wait a while and try again")
            }
            BrokerError::Unreachable(why) => write!(f, "could not reach the sign-in service: {why}"),
        }
    }
}

/// Percent-encode the few characters a URL query cannot carry raw.
///
/// A whole encoder for one value would be a dependency; this covers what a loopback URL
/// actually contains, which is a scheme, digits, dots, a colon and a slash.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending() -> Pending {
        begin("https://accounts.lightcone.example/", 7635, "NONCE1")
    }

    #[test]
    fn the_sign_in_url_carries_an_encoded_loopback_and_the_state() {
        let pending = pending();
        assert_eq!(pending.return_to, "http://127.0.0.1:7635/return");
        assert_eq!(
            pending.open,
            "https://accounts.lightcone.example/signin\
             ?return_to=http%3A%2F%2F127.0.0.1%3A7635%2Freturn&state=NONCE1",
        );
        // The colons and slashes are encoded, or the broker reads a truncated `return_to`.
        assert!(!pending.open.contains("://127.0.0.1"));
    }

    #[test]
    fn a_callback_yields_its_code() {
        let pending = pending();
        assert_eq!(
            code_from(&pending, "GET /return?code=ABC123&state=NONCE1 HTTP/1.1"),
            Ok("ABC123".into()),
        );
        // Order does not matter, and anything else in the query is ignored.
        assert_eq!(
            code_from(&pending, "GET /return?state=NONCE1&extra=x&code=ABC123 HTTP/1.1"),
            Ok("ABC123".into()),
        );
    }

    /// The reason `state` exists. Without this check an attacker completes a sign-in as
    /// themselves in someone else's client, and that someone plays the attacker's account
    /// without ever noticing.
    #[test]
    fn a_callback_for_somebody_elses_sign_in_is_refused() {
        let pending = pending();
        for hostile in [
            "GET /return?code=ABC123&state=SOMEONE-ELSE HTTP/1.1",
            "GET /return?code=ABC123 HTTP/1.1",
            "GET /return?code=ABC123&state= HTTP/1.1",
        ] {
            assert_eq!(code_from(&pending, hostile), Err(CallbackError::WrongState), "{hostile}");
        }
    }

    /// Anything encoded is refused rather than decoded: a decoder is a thing that can be
    /// tricked into producing a separator.
    #[test]
    fn an_encoded_callback_is_refused_rather_than_decoded() {
        let pending = pending();
        for hostile in [
            "GET /return?code=A%26state%3DX&state=NONCE1 HTTP/1.1",
            "GET /%72eturn?code=ABC&state=NONCE1 HTTP/1.1",
        ] {
            assert_eq!(code_from(&pending, hostile), Err(CallbackError::NotOurs), "{hostile}");
        }
    }

    #[test]
    fn a_request_for_anything_else_is_not_ours() {
        let pending = pending();
        for other in [
            "GET /favicon.ico HTTP/1.1",
            "GET /return/../evil?code=A&state=NONCE1 HTTP/1.1",
            // A cross-origin form can be made to POST anywhere. It is refused on the method,
            // before the state is even looked at.
            "POST /return?code=A&state=NONCE1 HTTP/1.1",
            "",
        ] {
            assert!(code_from(&pending, other).is_err(), "{other:?} was accepted");
        }
        assert_eq!(
            code_from(&pending, "POST /return?code=A&state=NONCE1 HTTP/1.1"),
            Err(CallbackError::NotOurs),
        );
        assert_eq!(code_from(&pending, "GET /return HTTP/1.1"), Err(CallbackError::NoCode));
    }

    #[test]
    fn the_landing_page_says_which_way_it_went_and_frames_nothing() {
        let good = landing_page(true);
        assert!(good.contains("200 OK") && good.contains("Signed in"));
        assert!(good.contains("no-referrer"), "the code is in the URL of this very page");
        assert!(landing_page(false).contains("did not complete"));
        // A correct Content-Length, or the browser waits for bytes that never come.
        let body = good.split("\r\n\r\n").nth(1).unwrap();
        assert!(good.contains(&format!("Content-Length: {}", body.len())));
    }

    /// The whole loop, over a real socket: bind, be redirected to, hand back the code.
    ///
    /// Worth doing end to end because the bug this found was not in the parsing. The sign-in
    /// used to reach the accepting thread through a thread-local, which that thread does not
    /// share, so every answer came back as somebody else's and no unit test could see it.
    #[test]
    fn a_browser_redirected_to_the_listener_delivers_its_code() {
        use std::io::Read;
        use std::net::TcpStream;

        let bound = Bound::open().expect("it binds");
        let port = bound.port;
        assert!(port > 0, "the operating system chose nothing");
        let pending = begin("https://accounts.lightcone.example", port, "NONCE1");
        assert!(pending.open.contains(&format!("%3A{port}%2F")), "{}", pending.open);

        let loopback = bound.listen(pending);
        let mut browser = TcpStream::connect(("127.0.0.1", port)).expect("connected");
        browser
            .write_all(b"GET /return?code=THE-CODE&state=NONCE1 HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();

        // The browser is left looking at something, rather than at a connection reset.
        let mut said = String::new();
        browser.read_to_string(&mut said).unwrap();
        assert!(said.contains("200 OK") && said.contains("Signed in"), "{said}");

        let answer = (0..200)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(5));
                loopback.poll()
            })
            .expect("no answer came back");
        assert_eq!(answer, Ok("THE-CODE".into()));
    }

    /// And somebody else's sign-in gets the failure page, not the code.
    #[test]
    fn a_callback_with_the_wrong_state_is_refused_over_the_socket() {
        use std::io::Read;
        use std::net::TcpStream;

        let bound = Bound::open().expect("it binds");
        let port = bound.port;
        let loopback = bound.listen(begin("https://accounts.lightcone.example", port, "OURS"));

        let mut browser = TcpStream::connect(("127.0.0.1", port)).expect("connected");
        browser
            .write_all(b"GET /return?code=THEIRS&state=NOT-OURS HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut said = String::new();
        browser.read_to_string(&mut said).unwrap();
        assert!(said.contains("did not complete"), "{said}");

        let answer = (0..200)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(5));
                loopback.poll()
            })
            .expect("no answer came back");
        assert_eq!(answer, Err(CallbackError::WrongState));
    }
}
