//! Pipe I/O to the Doubao server. The message format is in `crate::protocol::doubao`.

use anyhow::Result;

use super::pipe::Pipe;
use crate::protocol::doubao::{
    PIPE, RESPONSE_HEADER_LEN, get_state_request, parse_bool_body, parse_response_header,
    set_state_request,
};

/// Sends one request and returns the response body.
fn call(request: &[u8]) -> Result<Vec<u8>> {
    let pipe = Pipe::open(PIPE, "Doubao")?;
    pipe.write(request)?;
    let mut header = [0u8; RESPONSE_HEADER_LEN];
    pipe.read_exact(&mut header)?;
    let mut body = vec![0u8; parse_response_header(&header)?];
    pipe.read_exact(&mut body)?;
    Ok(body)
}

/// Whether Doubao is in Chinese mode.
pub fn is_chinese() -> Result<bool> {
    parse_bool_body(&call(&get_state_request())?)
}

/// Puts Doubao into Chinese or English mode. The server also updates its status-bar icon.
pub fn set_chinese(chinese: bool) -> Result<()> {
    call(&set_state_request(chinese)).map(|_| ())
}
