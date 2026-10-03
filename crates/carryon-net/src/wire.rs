//! Wire protocol data model (spec §18). Control messages are serde-serialized and
//! sent as length-delimited frames (see [`crate::frame`]) inside the TLS 1.3
//! channel (see [`crate::tls`]).
//!
//! # Boundary note
//!
//! The manifest that crosses the wire is exactly [`carryon_adapter_api::ObjectManifest`]
//! — the same type an adapter produces locally. The destination core re-hashes and
//! re-verifies every object's bytes against the manifest's declared `content_hash`
//! before publishing (CORE-004 holds over the wire, not just locally). The wire
//! carries **no executable paths, class names, or shell strings** (§3.8, NET /
//! no-RCE): a `TransferRequest` names a content digest, never a path.

use carryon_adapter_api::ObjectManifest;
use serde::{Deserialize, Serialize};

/// Protocol version (§18.2). Major mismatch fails closed; minor may interoperate.
pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 0;

/// Maximum control-frame length (§25.3: control frames bounded to 1 MiB).
pub const MAX_CONTROL_FRAME: u32 = 1024 * 1024;

/// Maximum payload chunk length carried in a `ChunkData` message (§25.3 bounded,
/// default near 1 MiB). The *object* may be larger; it is split into chunks.
pub const MAX_CHUNK_BYTES: u32 = 1024 * 1024;

/// The common envelope (§18.1). The TLS channel authenticates devices; these
/// fields provide ordering, replay detection, dedup, and audit — not security.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    /// Per-session monotonic sequence number (§18.1). The receiver rejects a
    /// repeat or a gap below the high-water mark (replay/sequence protection).
    pub sequence: u64,
    /// Unique message id for idempotency/audit.
    pub message_id: String,
    pub message: Message,
}

impl Envelope {
    /// Wrap a message with the current protocol version and a fresh id.
    pub fn new(sequence: u64, message: Message) -> Self {
        Envelope {
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: PROTOCOL_MINOR,
            sequence,
            message_id: uuid::Uuid::new_v4().to_string(),
            message,
        }
    }
}

/// A control message (§18). State-changing requests and their replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// Version/capability negotiation, sent first by the connecting peer (§18.2).
    Hello {
        /// Feature flags the peer advertises. Unknown flags are ignored.
        features: Vec<String>,
    },
    /// Reply to `Hello` confirming the negotiated version/features.
    Welcome { features: Vec<String> },

    /// Destination asks the source for a sealed cut's manifest (§18.6).
    CutRequest { session: String, cut_number: u64 },
    /// Source returns the manifest for the requested cut.
    CutManifest { manifest: ObjectManifest },

    /// Destination requests a byte range of one object, addressed **by content
    /// digest** (never by path). Resumable: it asks only for ranges it lacks (§18.6).
    TransferRequest {
        content_hash: String,
        offset: u64,
        length: u64,
    },
    /// Source returns the requested bytes plus the per-chunk digest (NET-007).
    ChunkData {
        content_hash: String,
        offset: u64,
        /// sha256 of `bytes`, so the receiver detects a conflicting duplicate.
        chunk_digest: String,
        #[serde(with = "b64")]
        bytes: Vec<u8>,
    },
    /// Source reports the object/range is not available (e.g. unknown digest).
    Unavailable {
        content_hash: String,
        reason: String,
    },

    /// Destination confirms it imported + verified the whole cut (source-off ok).
    ImportComplete {
        session: String,
        cut_number: u64,
        /// The cut's manifest digest, echoed back so the source can audit agreement.
        manifest_digest: String,
    },
    // --- Authority transfer (L4 single-writer, §21.2). ---
    // The handshake runs *after* the destination has imported and verified the
    // cut (so it already holds the authoritative state). Epochs are monotonic
    // (§21.3); the proposal binds the new epoch to the exact cut and to the
    // destination's pin, so a stale or misdirected proposal fails closed.
    /// Source → destination: offer authority for `cut_number` at `new_epoch`
    /// (step 1/5). `proposal_id` is opaque and echoed in later steps. The
    /// adapter-produced `proposal` blob is bound to the cut and destination.
    AuthorityProposal {
        session: String,
        cut_number: u64,
        /// Strictly greater than the source's current owned epoch (§21.3).
        new_epoch: u64,
        proposal_id: String,
        /// Opaque, adapter-produced proposal payload (§9 PrepareAuthorityTransfer).
        proposal: String,
    },
    /// Destination → source: durably accepted `new_epoch` and returns its
    /// acceptance receipt (steps 6–7). Presence of this receipt is what lets the
    /// source commit relinquishment (AUTH-003).
    AuthorityAccept {
        proposal_id: String,
        /// Durable destination receipt proving acceptance (AUTH-003).
        dest_receipt: String,
    },
    /// Source → destination: relinquishment committed; here is the final source
    /// receipt (step 8). On receipt the destination enters authoritative state
    /// (step 9). After sending this the source is a read-only replica.
    AuthorityCommit {
        proposal_id: String,
        /// Durable source receipt proving relinquishment (AUTH-003).
        source_receipt: String,
    },
    /// Either peer aborts the transfer. Authority does NOT move; the source keeps
    /// it. Network loss alone MUST NOT be read as relinquishment (AUTH-002).
    AuthorityAbort { proposal_id: String, reason: String },

    /// Either peer reports a fatal protocol error and closes.
    Abort { reason: String },
}

/// Compact base64 for chunk payloads inside JSON frames (no external dep).
mod b64 {
    use serde::{Deserialize, Deserializer, Serializer};

    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).map_err(serde::de::Error::custom)
    }

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
            out.push(CHARS[((n >> 18) & 63) as usize] as char);
            out.push(CHARS[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                CHARS[((n >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                CHARS[(n & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        fn val(c: u8) -> Result<u32, String> {
            match c {
                b'A'..=b'Z' => Ok((c - b'A') as u32),
                b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
                b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
                b'+' => Ok(62),
                b'/' => Ok(63),
                _ => Err("invalid base64 char".into()),
            }
        }
        let s = s.as_bytes();
        if !s.len().is_multiple_of(4) {
            return Err("base64 length not a multiple of 4".into());
        }
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        for chunk in s.chunks(4) {
            let pad = chunk.iter().filter(|&&c| c == b'=').count();
            let n = (val(chunk[0])? << 18)
                | (val(chunk[1])? << 12)
                | (if chunk[2] == b'=' { 0 } else { val(chunk[2])? } << 6)
                | (if chunk[3] == b'=' { 0 } else { val(chunk[3])? });
            out.push((n >> 16) as u8);
            if pad < 2 {
                out.push((n >> 8) as u8);
            }
            if pad < 1 {
                out.push(n as u8);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for case in [&b""[..], b"x", b"ab", b"abc", b"abcd", &[0u8, 255, 1, 2, 3]] {
            let e = b64::encode(case);
            assert_eq!(b64::decode(&e).unwrap(), case, "roundtrip {case:?}");
        }
    }

    #[test]
    fn base64_rejects_bad_length() {
        assert!(b64::decode("abc").is_err());
    }

    #[test]
    fn envelope_serde_roundtrip() {
        let env = Envelope::new(
            7,
            Message::TransferRequest {
                content_hash: "a".repeat(64),
                offset: 0,
                length: 1024,
            },
        );
        let json = serde_json::to_vec(&env).unwrap();
        let back: Envelope = serde_json::from_slice(&json).unwrap();
        assert_eq!(env, back);
    }
}
