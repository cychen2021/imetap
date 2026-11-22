//! Pipe I/O to the Mingkuai server (`mingkuaid`). The message format is in `crate::mingkuai`.

use anyhow::{Result, bail};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_MODE, OPEN_EXISTING, ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::core::HSTRING;

use crate::mingkuai::{
    LENGTH_LEN, get_mode_request, parse_length, parse_mode_reply, pipe_name, set_mode_request,
};

/// Closes the pipe handle on drop.
struct Pipe(HANDLE);

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateFileW` and is closed only here.
        unsafe { CloseHandle(self.0).ok() };
    }
}

fn open() -> Result<Pipe> {
    let name = HSTRING::from(format!(r"\\.\pipe\{}", pipe_name()));
    for _ in 0..2 {
        // SAFETY: `name` outlives the call. The handle is owned by `Pipe`.
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
                // SAFETY: `name` outlives the call.
                let _ = unsafe { WaitNamedPipeW(&name, 300) };
            }
            Err(e) => return Err(e.into()),
        }
    }
    bail!("the Mingkuai pipe stayed busy")
}

fn read_exact(pipe: &Pipe, buf: &mut [u8]) -> Result<()> {
    let mut done = 0;
    while done < buf.len() {
        let mut got = 0u32;
        // SAFETY: the buffer and the count outlive the synchronous call.
        unsafe { ReadFile(pipe.0, Some(&mut buf[done..]), Some(&mut got), None)? };
        if got == 0 {
            bail!("the Mingkuai server closed the pipe");
        }
        done += got as usize;
    }
    Ok(())
}

fn call(request: &[u8]) -> Result<Vec<u8>> {
    let pipe = open()?;
    let mut written = 0u32;
    // SAFETY: the request and the count outlive the synchronous call.
    unsafe { WriteFile(pipe.0, Some(request), Some(&mut written), None)? };
    let mut prefix = [0u8; LENGTH_LEN];
    read_exact(&pipe, &mut prefix)?;
    let mut body = vec![0u8; parse_length(prefix)?];
    read_exact(&pipe, &mut body)?;
    Ok(body)
}

/// Whether Mingkuai is in Chinese mode. The mode is the same in all applications.
pub fn is_chinese() -> Result<bool> {
    parse_mode_reply(&call(&get_mode_request())?)
}

/// Puts Mingkuai into Chinese or English mode, in all applications.
pub fn set_chinese(chinese: bool) -> Result<()> {
    parse_mode_reply(&call(&set_mode_request(chinese))?).map(|_| ())
}
