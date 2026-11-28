//! The client end of a named pipe, as the Doubao and Mingkuai servers offer one.

use anyhow::{Result, bail};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_MODE, OPEN_EXISTING, ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::core::HSTRING;

/// An open pipe, closed on drop.
pub struct Pipe {
    handle: HANDLE,
    /// The server's name, for error messages.
    server: &'static str,
}

impl Pipe {
    /// Opens the pipe at `path` (`\\.\pipe\...`). When the server is busy with another client, waits
    /// for it once.
    pub fn open(path: &str, server: &'static str) -> Result<Pipe> {
        let name = HSTRING::from(path);
        for _ in 0..2 {
            // SAFETY: `name` outlives the call. The handle is owned by the returned `Pipe`.
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
                Ok(handle) => return Ok(Pipe { handle, server }),
                Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                    // SAFETY: `name` outlives the call.
                    let _ = unsafe { WaitNamedPipeW(&name, 300) };
                }
                Err(e) => return Err(e.into()),
            }
        }
        bail!("the {server} pipe stayed busy")
    }

    /// Writes all of `data`.
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut written = 0u32;
        // SAFETY: the data and the count outlive the synchronous call.
        unsafe { WriteFile(self.handle, Some(data), Some(&mut written), None)? };
        Ok(())
    }

    /// Fills `buf`, reading as often as needed.
    pub fn read_exact(&self, buf: &mut [u8]) -> Result<()> {
        let mut done = 0;
        while done < buf.len() {
            let mut got = 0u32;
            // SAFETY: the buffer and the count outlive the synchronous call.
            unsafe { ReadFile(self.handle, Some(&mut buf[done..]), Some(&mut got), None)? };
            if got == 0 {
                bail!("the {} server closed the pipe", self.server);
            }
            done += got as usize;
        }
        Ok(())
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateFileW` and is closed only here.
        unsafe { CloseHandle(self.handle).ok() };
    }
}
