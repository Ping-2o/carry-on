//! Authenticated transport session (spec §8.6/§18.1/§18.5). A blocking TLS 1.3
//! connection over TCP carrying length-delimited control frames. Mutual pinning
//! is enforced in the handshake (see [`crate::tls`]); this layer adds per-session
//! monotonic sequence numbers with replay/gap rejection (§18.1, PROTO_Sequence/
//! Replay) and protocol-version checking (§18.2, fail closed on major mismatch).
//!
//! Blocking I/O matches the core's blocking API — no async runtime is pulled in.

use crate::error::{NetError, ProtoCode, Result};
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

    /// Total wire bytes sent on this session (length prefix + body). Measured, not
    /// estimated — for honest transfer-cost metrics.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent
    }

    /// Total wire bytes received on this session.
    pub fn bytes_recv(&self) -> u64 {
        self.bytes_recv
    }

    /// Send one message, stamping the next sequence number.
    pub fn send(&mut self, message: Message) -> Result<()> {
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
