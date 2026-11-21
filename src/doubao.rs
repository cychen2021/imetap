//! The wire protocol of Doubao IME's server (`ImeService`), reverse engineered from
//! `doubaoime-rpc-new.dll` v1.0.0.23.
//!
//! Doubao keeps its Chinese/English state and its status-bar icon in a server process, and exposes
//! the operations as RPC over the named pipe `\\.\pipe\ObricIme\oime-server`. Setting the state
//! through the pipe is the only way to change it from outside: the server's own keyboard hook
//! ignores injected keys. This module only builds and parses messages; the pipe I/O lives in
//! `win::doubao`.
//!
//! ```text
//! request : "OMPE"(u32 0x45504D4F) | version u16 = 1 | op u16 | body size u32 | body
//! response: "OMPE" | version u16 | op u16 | status u32 (0 = ok) | body size u32 | body
//! op 13  GetInputState(Int32Type{value})                 -> BoolType{value}
//! op 14  SetInputState(InputState{input_type, state})
//! ```
//!
//! Bodies are protobuf: field 1 varint is `08 xx`, field 2 bool is `10 xx`. `input_type` 1 is the
//! Chinese/English mode.

use anyhow::{Result, bail};

/// Named pipe of the Doubao server.
pub const PIPE: &str = r"\\.\pipe\ObricIme\oime-server";

const MAGIC: u32 = 0x4550_4D4F;
const VERSION: u16 = 1;
const OP_GET_INPUT_STATE: u16 = 13;
const OP_SET_INPUT_STATE: u16 = 14;
/// `input_type` of the Chinese/English mode.
const INPUT_TYPE_CN_EN: u8 = 1;

/// Size of a request header in bytes.
pub const REQUEST_HEADER_LEN: usize = 12;
/// Size of a response header in bytes.
pub const RESPONSE_HEADER_LEN: usize = 16;

/// A complete request asking for the Chinese/English state.
pub fn get_state_request() -> Vec<u8> {
    request(OP_GET_INPUT_STATE, &[0x08, INPUT_TYPE_CN_EN])
}

/// A complete request setting the Chinese/English state.
pub fn set_state_request(chinese: bool) -> Vec<u8> {
    request(
        OP_SET_INPUT_STATE,
        &[0x08, INPUT_TYPE_CN_EN, 0x10, chinese as u8],
    )
}

fn request(op: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(REQUEST_HEADER_LEN + body.len());
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Validates a response header and returns the size of the body that follows.
pub fn parse_response_header(header: &[u8; RESPONSE_HEADER_LEN]) -> Result<usize> {
    let word = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    if word(0) != MAGIC {
        bail!("not a Doubao response (bad magic {:#x})", word(0));
    }
    if word(8) != 0 {
        bail!("Doubao server returned status {}", word(8));
    }
    Ok(word(12) as usize)
}

/// Decodes a `BoolType{value}` body. An empty body is protobuf for `false`.
pub fn parse_bool_body(body: &[u8]) -> Result<bool> {
    match body {
        [] => Ok(false),
        [0x08, v, ..] => Ok(*v != 0),
        _ => bail!("unexpected BoolType body {body:02x?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_request_layout() {
        assert_eq!(
            get_state_request(),
            [0x4F, 0x4D, 0x50, 0x45, 1, 0, 13, 0, 2, 0, 0, 0, 0x08, 1]
        );
    }

    #[test]
    fn set_request_carries_the_state() {
        assert_eq!(set_state_request(true)[12..], [0x08, 1, 0x10, 1]);
        assert_eq!(set_state_request(false)[12..], [0x08, 1, 0x10, 0]);
        assert_eq!(&set_state_request(true)[6..8], &[14, 0]);
    }

    #[test]
    fn response_header_gives_the_body_size() {
        let mut h = [0u8; RESPONSE_HEADER_LEN];
        h[..4].copy_from_slice(&MAGIC.to_le_bytes());
        h[12..].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(parse_response_header(&h).unwrap(), 2);
    }

    #[test]
    fn a_bad_magic_or_status_is_an_error() {
        let mut h = [0u8; RESPONSE_HEADER_LEN];
        assert!(parse_response_header(&h).is_err());
        h[..4].copy_from_slice(&MAGIC.to_le_bytes());
        h[8] = 1;
        assert!(parse_response_header(&h).is_err());
    }

    #[test]
    fn bool_bodies() {
        assert!(parse_bool_body(&[0x08, 1]).unwrap());
        assert!(!parse_bool_body(&[0x08, 0]).unwrap());
        assert!(!parse_bool_body(&[]).unwrap());
        assert!(parse_bool_body(&[0x10, 1]).is_err());
    }
}
