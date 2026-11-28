//! The wire format of the Mingkuai server (`mingkuaid`), for reading and setting its
//! Chinese/English mode.
//!
//! The support is hidden: it is on only when `mingkuai-integrate.enable` is true in
//! `config.toml`. When it is off, Mingkuai counts as an unknown input method.
//!
//! ```text
//! message : length u32 little-endian | that many bytes of JSON
//! request : "get_mode"                       -> {"mode":{"chinese":true}}
//! request : {"set_mode":{"chinese":false}}   -> {"mode":{"chinese":false}}
//! request : "watch_mode"                     -> {"mode":{...}} now and after each change
//! error   : {"error":{"message":"..."}}
//! ```
//!
//! The mode is one mode for all applications. This module only builds and parses messages.
//! The pipe I/O lives in `win::mingkuai`, and the socket I/O of macOS in `mac::mingkuai`.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, bail};
use serde::Deserialize;

/// Size of the length before each message, in bytes.
pub const LENGTH_LEN: usize = 4;
/// Replies longer than this are an error. The server limit is 1 MiB.
pub const MAX_REPLY_LEN: usize = 1 << 20;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Turns the Mingkuai support on or off, from `mingkuai-integrate.enable`.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether the Mingkuai support is on.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The pipe name of the server of the current user, without the platform prefix.
pub fn pipe_name() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    format!("mingkuai-{user}")
}

/// The socket file of the server of the current user on macOS. The server lives in the Application
/// Support folder of the user, with the permissions of the user only.
#[cfg(unix)]
pub fn socket_path() -> Result<std::path::PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(std::path::PathBuf::from(home).join("Library/Application Support/Mingkuai/mingkuaid.sock"))
}

/// A complete request asking for the Chinese/English mode.
pub fn get_mode_request() -> Vec<u8> {
    frame(br#""get_mode""#)
}

/// A complete request to get the mode now and after each change. The connection then takes no
/// more requests.
pub fn watch_mode_request() -> Vec<u8> {
    frame(br#""watch_mode""#)
}

/// A complete request setting the Chinese/English mode.
pub fn set_mode_request(chinese: bool) -> Vec<u8> {
    frame(format!(r#"{{"set_mode":{{"chinese":{chinese}}}}}"#).as_bytes())
}

fn frame(json: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(LENGTH_LEN + json.len());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(json);
    out
}

/// The size of the reply body that follows a length prefix.
pub fn parse_length(prefix: [u8; LENGTH_LEN]) -> Result<usize> {
    let len = u32::from_le_bytes(prefix) as usize;
    if len > MAX_REPLY_LEN {
        bail!("Mingkuai reply is too large ({len} bytes)");
    }
    Ok(len)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reply {
    Mode { chinese: bool },
    Error { message: String },
}

/// Decodes a reply to `get_mode` or `set_mode`: true for Chinese mode.
pub fn parse_mode_reply(body: &[u8]) -> Result<bool> {
    match serde_json::from_slice(body) {
        Ok(Reply::Mode { chinese }) => Ok(chinese),
        Ok(Reply::Error { message }) => bail!("Mingkuai server failed: {message}"),
        Err(_) => bail!(
            "unexpected Mingkuai reply {}",
            String::from_utf8_lossy(body)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_framed_json() {
        assert_eq!(get_mode_request(), b"\x0a\0\0\0\"get_mode\"");
        assert_eq!(&watch_mode_request()[4..], br#""watch_mode""#);
        let set = set_mode_request(false);
        assert_eq!(&set[4..], br#"{"set_mode":{"chinese":false}}"#);
        assert_eq!(
            parse_length(set[..4].try_into().unwrap()).unwrap(),
            set.len() - 4
        );
    }

    #[test]
    fn mode_replies_parse() {
        assert!(parse_mode_reply(br#"{"mode":{"chinese":true}}"#).unwrap());
        assert!(!parse_mode_reply(br#"{"mode":{"chinese":false}}"#).unwrap());
        assert!(parse_mode_reply(br#"{"error":{"message":"no"}}"#).is_err());
        assert!(parse_mode_reply(br#""done""#).is_err());
    }

    #[test]
    fn too_large_replies_are_rejected() {
        assert!(parse_length((MAX_REPLY_LEN as u32 + 1).to_le_bytes()).is_err());
    }
}
