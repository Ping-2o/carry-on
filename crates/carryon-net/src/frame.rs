//! Length-delimited framing (spec §8.6 "length-delimited control messages").
//!
//! Each frame on the wire is a big-endian `u32` length followed by exactly that
//! many bytes of a serde-serialized [`crate::wire::Envelope`]. The length is
//! bounded by [`crate::wire::MAX_CONTROL_FRAME`]; an oversized or truncated frame
//! fails closed with `PROTO_Framing` (decompression/allocation-bomb guard, §22.2).

use crate::error::{NetError, ProtoCode, Result};
use crate::wire::{Envelope, MAX_CONTROL_FRAME};
use std::io::{Read, Write};

/// Serialize `env` and write one length-delimited frame to `w`. Returns the total
/// bytes written on the wire (4-byte length prefix + body), for byte accounting.
pub fn write_frame<W: Write>(w: &mut W, env: &Envelope) -> Result<u64> {
    let body = serde_json::to_vec(env)?;
    if body.len() as u64 > MAX_CONTROL_FRAME as u64 {
        return Err(NetError::proto(
            ProtoCode::Framing,
            format!("frame {} bytes exceeds cap {MAX_CONTROL_FRAME}", body.len()),
        ));
    }
    w.write_all(&(body.len() as u32).to_be_bytes())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(4 + body.len() as u64)
}

/// Read one length-delimited frame from `r`. Rejects a length over the cap
/// before allocating (§22.2 bounded allocation). An EOF at the frame boundary
/// is reported as `PROTO_Framing` "closed".
pub fn read_frame<R: Read>(r: &mut R) -> Result<(Envelope, u64)> {
    let mut len_buf = [0u8; 4];
    read_exact_or_closed(r, &mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_CONTROL_FRAME {
        return Err(NetError::proto(
            ProtoCode::Framing,
            format!("declared frame {len} exceeds cap {MAX_CONTROL_FRAME}"),
        ));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)
        .map_err(|e| NetError::proto(ProtoCode::Framing, format!("truncated frame body: {e}")))?;
    let env: Envelope = serde_json::from_slice(&body)
        .map_err(|e| NetError::proto(ProtoCode::Framing, format!("unparseable frame: {e}")))?;
    Ok((env, 4 + len as u64))
}

/// Read exactly `buf.len()` bytes, mapping a clean EOF to a closed-connection error.
fn read_exact_or_closed<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<()> {
    match r.read_exact(buf) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            Err(NetError::proto(ProtoCode::Framing, "connection closed"))
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Message;

    #[test]
    fn frame_roundtrip_over_buffer() {
        let env = Envelope::new(1, Message::Hello { features: vec![] });
        let mut buf = Vec::new();
        write_frame(&mut buf, &env).unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        assert_eq!(read_frame(&mut cursor).unwrap().0, env);
    }

    #[test]
    fn oversized_declared_length_rejected_before_alloc() {
        // A frame claiming 4 GiB must fail closed without allocating.
        let mut evil = (u32::MAX).to_be_bytes().to_vec();
        evil.extend_from_slice(b"junk");
        let mut cursor = std::io::Cursor::new(evil);
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.family(), "PROTO");
    }

    #[test]
    fn truncated_body_fails_closed() {
        let env = Envelope::new(1, Message::Hello { features: vec![] });
        let mut buf = Vec::new();
        write_frame(&mut buf, &env).unwrap();
        buf.truncate(buf.len() - 1); // chop last body byte
        let mut cursor = std::io::Cursor::new(buf);
        assert!(read_frame(&mut cursor).is_err());
    }

    #[test]
    fn closed_at_boundary_reported() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        let err = read_frame(&mut cursor).unwrap_err();
        assert_eq!(err.family(), "PROTO");
    }
}
