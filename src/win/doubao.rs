//! Pipe I/O to the Doubao server; the message format is in `crate::doubao`.

use anyhow::{Result, bail};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_MODE, OPEN_EXISTING, ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::core::HSTRING;

use crate::doubao::{
    PIPE, RESPONSE_HEADER_LEN, get_state_request, parse_bool_body, parse_response_header,
    set_state_request,
};

/// Closes the pipe handle on drop.
struct Pipe(HANDLE);

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` and is closed exactly once.
        unsafe { CloseHandle(self.0).ok() };
    }
}

fn open() -> Result<Pipe> {
    let name = HSTRING::from(PIPE);
    for _ in 0..2 {
        // SAFETY: `name` outlives the call; the returned handle is owned by `Pipe`.
        let opened = unsafe {
            CreateFileW(
                &name,
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };
        match opened {
            Ok(handle) => return Ok(Pipe(handle)),
            Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                // SAFETY: waiting on a named pipe by name has no preconditions.
                let _ = unsafe { WaitNamedPipeW(&name, 300) };
            }
            Err(e) => return Err(e.into()),
        }
    }
    bail!("the Doubao pipe stayed busy")
}

fn read_exact(pipe: &Pipe, buf: &mut [u8]) -> Result<()> {
    let mut got = 0u32;
    // SAFETY: `buf` is valid for `buf.len()` bytes for the duration of the call.
    unsafe { ReadFile(pipe.0, Some(buf), Some(&mut got), None)? };
    if got as usize != buf.len() {
        bail!(
            "short read from the Doubao pipe ({got} of {} bytes)",
            buf.len()
        );
    }
    Ok(())
}

/// Sends one request and returns the response body.
fn call(request: &[u8]) -> Result<Vec<u8>> {
    let pipe = open()?;
    let mut written = 0u32;
    // SAFETY: `request` is valid for the duration of the call.
    unsafe { WriteFile(pipe.0, Some(request), Some(&mut written), None)? };
    let mut header = [0u8; RESPONSE_HEADER_LEN];
    read_exact(&pipe, &mut header)?;
    let mut body = vec![0u8; parse_response_header(&header)?];
    if !body.is_empty() {
        read_exact(&pipe, &mut body)?;
    }
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
