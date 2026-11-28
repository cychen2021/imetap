//! Pipe I/O to the Mingkuai server (`mingkuaid`). The message format is in `crate::protocol::mingkuai`.

use anyhow::Result;

use super::pipe::Pipe;
use crate::protocol::mingkuai::{
    LENGTH_LEN, get_mode_request, parse_length, parse_mode_reply, pipe_name, set_mode_request,
    watch_mode_request,
};

fn open() -> Result<Pipe> {
    Pipe::open(&format!(r"\\.\pipe\{}", pipe_name()), "Mingkuai")
}

fn read_reply(pipe: &Pipe) -> Result<Vec<u8>> {
    let mut prefix = [0u8; LENGTH_LEN];
    pipe.read_exact(&mut prefix)?;
    let mut body = vec![0u8; parse_length(prefix)?];
    pipe.read_exact(&mut body)?;
    Ok(body)
}

fn call(request: &[u8]) -> Result<Vec<u8>> {
    let pipe = open()?;
    pipe.write(request)?;
    read_reply(&pipe)
}

/// Calls `on_mode` with the mode now and after each change, until the connection fails. It blocks,
/// so run it on its own thread.
pub fn watch(mut on_mode: impl FnMut(bool)) -> Result<()> {
    let pipe = open()?;
    pipe.write(&watch_mode_request())?;
    loop {
        on_mode(parse_mode_reply(&read_reply(&pipe)?)?);
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
