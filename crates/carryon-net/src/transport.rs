//! Authenticated transport session (spec §8.6/§18.1/§18.5). A blocking TLS 1.3
//! connection over TCP carrying length-delimited control frames. Mutual pinning
//! is enforced in the handshake (see [`crate::tls`]); this layer adds per-session
//! monotonic sequence numbers with replay/gap rejection (§18.1, PROTO_Sequence/
//! Replay) and protocol-version checking (§18.2, fail closed on major mismatch).
//!
//! Blocking I/O matches the core's blocking API — no async runtime is pulled in.

use crate::error::{AuthCode, NetError, ProtoCode, Result};
use crate::frame::{read_frame, write_frame};
use crate::identity::{DeviceIdentity, Pin};
use crate::tls::{self, PeerPin};
use crate::trust::TrustStore;
use crate::wire::{Envelope, Message, PROTOCOL_MAJOR};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;

/// Bench-only simulated link latency. When `CARRYON_NET_DELAY_MS` is set to a
/// positive integer, each sent frame is delayed by that many milliseconds. This is
/// off by default (no env var = no delay) and exists so the preparation-strategy
/// benchmark can sweep a latency axis without root `tc`/netem. It inflates real
/// wall-clock only; the byte counters still measure real wire bytes. NOT a protocol
/// feature — never relied on for correctness.
fn inject_link_delay() {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Parse once (sentinel u64::MAX = "not yet read"), then reuse — the env var does
    // not change within a process run.
    static DELAY_MS: AtomicU64 = AtomicU64::new(u64::MAX);
    let mut ms = DELAY_MS.load(Ordering::Relaxed);
    if ms == u64::MAX {
        ms = std::env::var("CARRYON_NET_DELAY_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        DELAY_MS.store(ms, Ordering::Relaxed);
    }
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

/// A TLS stream in either role. `StreamOwned` is generic over the connection.
enum Tls {
    Client(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
    Server(rustls::StreamOwned<rustls::ServerConnection, TcpStream>),
}

impl Read for Tls {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Tls::Client(s) => s.read(buf),
            Tls::Server(s) => s.read(buf),
        }
    }
}

impl Write for Tls {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Tls::Client(s) => s.write(buf),
            Tls::Server(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Tls::Client(s) => s.flush(),
            Tls::Server(s) => s.flush(),
        }
    }
}

/// An authenticated transport session over a pinned TLS 1.3 connection.
pub struct Session {
    tls: Tls,
    peer_pin: Pin,
    /// Provisional pairing channel (§18.4.3): the peer pin was recorded, not
    /// yet trusted — pairing traffic only, content transfer is refused.
    pairing: bool,
    /// Next sequence number we will send.
    send_seq: u64,
    /// Highest sequence number we have accepted from the peer (replay guard).
    recv_high_water: Option<u64>,
    /// Total wire bytes sent on this session (length prefix + body), for metrics.
    bytes_sent: u64,
    /// Total wire bytes received on this session, for metrics.
    bytes_recv: u64,
}

impl Session {
    /// Connect to `addr` as the client, pinning the server against `trust`.
    /// Drives the TLS handshake to completion so a pin mismatch fails here.
    pub fn connect(
        addr: impl ToSocketAddrs,
        id: &DeviceIdentity,
        trust: Arc<TrustStore>,
    ) -> Result<Session> {
        let (cfg, verifier) = tls::client_config(id, trust)?;
        let tcp = TcpStream::connect(addr)?;
        // The SNI name is cosmetic here; trust comes from the pin, not the name.
        let server_name = rustls::pki_types::ServerName::try_from("peer.carryon.local")
            .map_err(|e| NetError::proto(ProtoCode::State, format!("server name: {e}")))?;
        let conn = rustls::ClientConnection::new(cfg, server_name)
            .map_err(|e| NetError::proto(ProtoCode::State, format!("client conn: {e}")))?;
        let mut stream = rustls::StreamOwned::new(conn, tcp);
        complete_handshake(&mut stream)?;
        let peer_pin = require_peer_pin(verifier)?;
        Ok(Session {
            tls: Tls::Client(stream),
            peer_pin,
            pairing: false,
            send_seq: 0,
            recv_high_water: None,
            bytes_sent: 0,
            bytes_recv: 0,
        })
    }

    /// Accept one connection from `listener` as the server, pinning the client.
    pub fn accept(
        listener: &TcpListener,
        id: &DeviceIdentity,
        trust: Arc<TrustStore>,
    ) -> Result<Session> {
        let (cfg, verifier) = tls::server_config(id, trust)?;
        let (tcp, _addr) = listener.accept()?;
        let conn = rustls::ServerConnection::new(cfg)
            .map_err(|e| NetError::proto(ProtoCode::State, format!("server conn: {e}")))?;
        let mut stream = rustls::StreamOwned::new(conn, tcp);
        complete_handshake(&mut stream)?;
        let peer_pin = require_peer_pin(verifier)?;
        Ok(Session {
            tls: Tls::Server(stream),
            peer_pin,
            pairing: false,
            send_seq: 0,
            recv_high_water: None,
            bytes_sent: 0,
            bytes_recv: 0,
        })
    }

    /// Connect as the client over the **provisional pairing channel**
    /// (§18.4.3): TLS 1.3 with full signature verification, but the server's
    /// pin is recorded, not pre-trusted. The trust decision happens after the
    /// user confirms the verification string (§18.4.4-5); content transfer on
    /// this session is refused (§18.5) — pair, then reconnect with the pinned
    /// trust store.
    pub fn connect_pairing(addr: impl ToSocketAddrs, id: &DeviceIdentity) -> Result<Session> {
        let (cfg, verifier) = tls::pairing_client_config(id)?;
        let tcp = TcpStream::connect(addr)?;
        let server_name = rustls::pki_types::ServerName::try_from("peer.carryon.local")
            .map_err(|e| NetError::proto(ProtoCode::State, format!("server name: {e}")))?;
        let conn = rustls::ClientConnection::new(cfg, server_name)
            .map_err(|e| NetError::proto(ProtoCode::State, format!("client conn: {e}")))?;
        let mut stream = rustls::StreamOwned::new(conn, tcp);
        complete_handshake(&mut stream)?;
        let peer_pin = verifier.accepted_pin().ok_or_else(|| {
            NetError::auth(AuthCode::Pairing, "pairing handshake recorded no peer pin")
        })?;
        Ok(Session {
            tls: Tls::Client(stream),
            peer_pin,
            pairing: true,
            send_seq: 0,
            recv_high_water: None,
            bytes_sent: 0,
            bytes_recv: 0,
        })
    }

    /// Accept one connection on the **provisional pairing channel** (§18.4.3):
    /// the connecting client's pin is recorded for the pairing exchange, not
    /// pre-trusted; content transfer on this session is refused (§18.5).
    pub fn accept_pairing(listener: &TcpListener, id: &DeviceIdentity) -> Result<Session> {
        let (cfg, verifier) = tls::pairing_server_config(id)?;
        let (tcp, _addr) = listener.accept()?;
        let conn = rustls::ServerConnection::new(cfg)
            .map_err(|e| NetError::proto(ProtoCode::State, format!("server conn: {e}")))?;
        let mut stream = rustls::StreamOwned::new(conn, tcp);
        complete_handshake(&mut stream)?;
        let peer_pin = verifier.accepted_pin().ok_or_else(|| {
            NetError::auth(AuthCode::Pairing, "pairing handshake recorded no peer pin")
        })?;
        Ok(Session {
            tls: Tls::Server(stream),
            peer_pin,
            pairing: true,
            send_seq: 0,
            recv_high_water: None,
            bytes_sent: 0,
            bytes_recv: 0,
        })
    }

    /// The pin of the peer authenticated on this session.
    pub fn peer_pin(&self) -> Pin {
        self.peer_pin
    }

    /// Whether this is a provisional pairing channel (§18.4.3). Pairing
    /// traffic only; a content operation on it is refused (§18.5).
    pub fn is_pairing(&self) -> bool {
        self.pairing
    }

    /// Total wire bytes sent on this session (length prefix + body). Measured, not
    /// estimated — for honest transfer-cost metrics.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }

    /// Total wire bytes received on this session.
    pub fn bytes_recv(&self) -> u64 {
        self.bytes_recv
    }

    /// Send one message, stamping the next sequence number. On a provisional
    /// pairing channel only negotiation (Hello/Welcome) may flow — any content
    /// message is refused closed (§18.4.3 → §18.5: content needs a pinned
    /// connection).
    pub fn send(&mut self, message: Message) -> Result<()> {
        if self.pairing && !matches!(message, Message::Hello { .. } | Message::Welcome { .. }) {
            return Err(NetError::auth(
                AuthCode::Pairing,
                "provisional pairing channel carries pairing traffic only — pair, then reconnect with the pinned trust (§18.4/§18.5)",
            ));
        }
        inject_link_delay();
        let env = Envelope::new(self.send_seq, message);
        let n = write_frame(&mut self.tls, &env)?;
        self.bytes_sent += n;
        self.send_seq += 1;
        Ok(())
    }

    /// Receive one message, enforcing version and strictly-increasing sequence
    /// (replay/gap → fail closed, §18.1).
    pub fn recv(&mut self) -> Result<Message> {
        let (env, n) = read_frame(&mut self.tls)?;
        self.bytes_recv += n;
        if env.protocol_major != PROTOCOL_MAJOR {
            return Err(NetError::proto(
                ProtoCode::Version,
                format!("peer major {} != ours {PROTOCOL_MAJOR}", env.protocol_major),
            ));
        }
        match self.recv_high_water {
            None => {}
            Some(hw) if env.sequence == hw + 1 => {}
            Some(hw) if env.sequence <= hw => {
                return Err(NetError::proto(
                    ProtoCode::Replay,
                    format!(
                        "sequence {} replays at-or-below high-water {hw}",
                        env.sequence
                    ),
                ));
            }
            Some(hw) => {
                return Err(NetError::proto(
                    ProtoCode::Sequence,
                    format!("sequence {} skips past high-water {hw}", env.sequence),
                ));
            }
        }
        self.recv_high_water = Some(env.sequence);
        Ok(env.message)
    }

    /// Client handshake step: send `Hello`, expect `Welcome` (§18.2).
    pub fn client_negotiate(&mut self, features: Vec<String>) -> Result<Vec<String>> {
        self.send(Message::Hello { features })?;
        match self.recv()? {
            Message::Welcome { features } => Ok(features),
            other => Err(NetError::proto(
                ProtoCode::State,
                format!("expected Welcome, got {other:?}"),
            )),
        }
    }

    /// Server handshake step: expect `Hello`, reply `Welcome` (§18.2).
    pub fn server_negotiate(&mut self, features: Vec<String>) -> Result<Vec<String>> {
        match self.recv()? {
            Message::Hello { features: theirs } => {
                self.send(Message::Welcome { features })?;
                Ok(theirs)
            }
            other => Err(NetError::proto(
                ProtoCode::State,
                format!("expected Hello, got {other:?}"),
            )),
        }
    }
}

/// Drive the TLS handshake to completion on a blocking stream so handshake
/// failures (including pin mismatch) surface immediately rather than on first I/O.
/// Generic over the two `StreamOwned` roles.
fn complete_handshake<S: Write>(stream: &mut S) -> Result<()> {
    // A flush forces rustls to process the handshake; map a handshake rejection
    // (e.g. pin mismatch in the verifier) to a closed error.
    stream
        .flush()
        .map_err(|e| NetError::proto(ProtoCode::State, format!("handshake: {e}")))
}

/// Extract the pin the verifier accepted; a missing pin means the handshake did
/// not actually authenticate a trusted peer (fail closed).
fn require_peer_pin(verifier: Arc<dyn PeerPin>) -> Result<Pin> {
    verifier.accepted_pin().ok_or_else(|| {
        NetError::auth(
            crate::error::AuthCode::Unpaired,
            "handshake completed without an accepted peer pin",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Message;
    use std::net::SocketAddr;

    fn bind_loopback() -> (TcpListener, SocketAddr) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        (l, addr)
    }

    /// §18.4.3: the provisional channel completes TLS 1.3 with real signature
    /// verification, both sides see the other's pin, negotiation flows — and
    /// content is refused closed on both directions (§18.5).
    #[test]
    fn provisional_pairing_channel_negotiates_but_refuses_content() {
        let (listener, addr) = bind_loopback();
        let src = DeviceIdentity::generate("src").unwrap();
        let dst = DeviceIdentity::generate("dst").unwrap();
        let dst_thread = dst.clone();

        let server = std::thread::spawn(move || Session::accept_pairing(&listener, &dst_thread));
        let mut client = Session::connect_pairing(addr, &src).unwrap();
        let mut server = server.join().unwrap().unwrap();

        assert!(client.is_pairing() && server.is_pairing());
        // Each side sees the TRUE pin of the other (recorded, not trusted).
        assert_eq!(client.peer_pin(), dst.pin());
        assert_eq!(server.peer_pin(), src.pin());

        // Negotiation (pairing traffic) works — request/response, so the two
        // halves must run concurrently (the same discipline as accept above).
        // The server thread then attempts CONTENT, which must be refused
        // closed on its side too (§18.5).
        let srv = std::thread::spawn(move || {
            let theirs = server.server_negotiate(vec!["pair".into()]).unwrap();
            let err = server
                .send(Message::ImportComplete {
                    session: "s".into(),
                    cut_number: 1,
                    manifest_digest: "d".into(),
                })
                .unwrap_err();
            (theirs, err.to_string())
        });
        let theirs = client.client_negotiate(vec!["pair".into()]).unwrap();
        let (srv_theirs, srv_err) = srv.join().unwrap();
        assert_eq!(theirs, srv_theirs);

        // Content is refused on BOTH directions, fail closed (§18.5).
        let err = client
            .send(Message::CutRequest {
                session: "s".into(),
                cut_number: 1,
            })
            .unwrap_err();
        assert!(err.to_string().contains("pairing traffic only"), "{err}");
        assert!(srv_err.contains("pairing traffic only"), "{srv_err}");
    }

    /// §18.4.6-7 → §18.5: after the user confirms, each side pins the other;
    /// the pinned reconnect succeeds; a REVOKED peer is refused (§18.4.9).
    #[test]
    fn pair_then_pinned_reconnect_then_revocation_blocks() {
        let (listener, addr) = bind_loopback();
        let src = DeviceIdentity::generate("src").unwrap();
        let dst = DeviceIdentity::generate("dst").unwrap();

        // Pair over the provisional channel (user confirm modeled as the VS match).
        let dst_thread = dst.clone();
        let server = std::thread::spawn(move || Session::accept_pairing(&listener, &dst_thread));
        let client = Session::connect_pairing(addr, &src).unwrap();
        let server = server.join().unwrap().unwrap();
        assert_eq!(
            crate::pairing::verification_string(&client.peer_pin(), &src.pin()),
            crate::pairing::verification_string(&server.peer_pin(), &dst.pin())
        );

        // Pin each side's store (what the shells persist in secure storage).
        let mut client_trust = TrustStore::new();
        client_trust.pair("dst", client.peer_pin(), "t");
        let mut server_trust = TrustStore::new();
        server_trust.pair("src", server.peer_pin(), "t");

        // A pinned reconnect succeeds (normal §18.5 path).
        let (l2, addr2) = bind_loopback();
        let dst2 = dst.clone();
        let st2 = server_trust.clone();
        let srv = std::thread::spawn(move || Session::accept(&l2, &dst2, Arc::new(st2)));
        let ok_client = Session::connect(addr2, &src, Arc::new(client_trust.clone())).unwrap();
        let ok_server = srv.join().unwrap().unwrap();
        drop((ok_client, ok_server));

        // §18.4.9: revoke the source; its reconnect is REFUSED (fail closed,
        // same semantics as unpaired_peer_fails_closed: the rejection surfaces
        // on the connection — connect or first I/O).
        assert!(server_trust.revoke(&src.pin()));
        let (l3, addr3) = bind_loopback();
        let dst3 = dst.clone();
        let st3 = server_trust.clone();
        let srv = std::thread::spawn(move || {
            Session::accept(&l3, &dst3, Arc::new(st3))
                .and_then(|mut s| s.recv().map(|_| ()))
                .is_err()
        });
        let refused = Session::connect(addr3, &src, Arc::new(client_trust))
            .and_then(|mut c| c.send(Message::Hello { features: vec![] }).map(|_| c))
            .and_then(|mut c| c.recv().map(|_| ()));
        assert!(refused.is_err(), "revoked peer must be refused");
        assert!(srv.join().unwrap(), "server must reject revoked client");
    }
}
