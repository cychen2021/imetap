//! Socket I/O to the Mingkuai server (`mingkuaid`). The message format is in `crate::protocol::mingkuai`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::protocol::mingkuai::{
    LENGTH_LEN, get_mode_request, parse_length, parse_mode_reply, set_mode_request, socket_path,
    watch_mode_request,
};

/// How long a call waits for the server. The server answers at once, so this only bounds a hung one.
const CALL_TIMEOUT: Duration = Duration::from_millis(500);

fn open() -> Result<UnixStream> {
    let path = socket_path()?;
    UnixStream::connect(&path).with_context(|| format!("cannot connect to {}", path.display()))
}

fn read_reply(stream: &mut UnixStream) -> Result<Vec<u8>> {
    let mut prefix = [0u8; LENGTH_LEN];
    stream.read_exact(&mut prefix)?;
    let mut body = vec![0u8; parse_length(prefix)?];
    stream.read_exact(&mut body)?;
    Ok(body)
}

fn call(request: &[u8]) -> Result<Vec<u8>> {
    let mut stream = open()?;
    stream.set_read_timeout(Some(CALL_TIMEOUT))?;
    stream.set_write_timeout(Some(CALL_TIMEOUT))?;
    stream.write_all(request)?;
    read_reply(&mut stream)
}

/// Calls `on_mode` with the mode now and after each change, until the connection fails. It blocks,
/// so run it on its own thread.
pub fn watch(mut on_mode: impl FnMut(bool)) -> Result<()> {
    let mut stream = open()?;
    stream.write_all(&watch_mode_request())?;
    loop {
        on_mode(parse_mode_reply(&read_reply(&mut stream)?)?);
    }
}

/// Whether Mingkuai is in Chinese mode. The mode is the same in all applications.
pub fn is_chinese() -> Result<bool> {
    parse_mode_reply(&call(&get_mode_request())?)
}

/// Puts Mingkuai into Chinese or English mode, in all applications.
pub fn set_chinese(chinese: bool) -> Result<()> {
    parse_mode_reply(&call(&set_mode_request(chinese))?).map(|_| ())
}
