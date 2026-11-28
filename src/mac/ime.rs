//! Which input source is which input method, and the last Chinese one used.
//!
//! The Chinese input methods are separate input sources, so the Ctrl double-tap swaps between the
//! US source and the Chinese one used last, which is kept in a file next to the config.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::ffi;
use crate::config::config_path;
use crate::protocol::mingkuai as mingkuai_wire;
use crate::state::Ime;

const SOURCE_US: &str = "com.apple.keylayout.US";
const SOURCE_DOUBAO: &str = "com.bytedance.inputmethod.doubaoime.pinyin";
const SOURCE_SHUANGPIN: &str = "com.apple.inputmethod.SCIM.Shuangpin";
/// The input mode of Mingkuai. Known only when `mingkuai-integrate.enable` is set.
const SOURCE_MINGKUAI: &str = "org.mingkuai.inputmethod.Mingkuai.Pinyin";

/// The input methods imetap supports on macOS that are enabled input sources (in the input menu).
/// When the sources cannot be listed, all of them count as enabled.
pub(super) fn available() -> Vec<Ime> {
    let enabled = ffi::enabled_sources();
    [
        (Ime::Us, SOURCE_US),
        (Ime::Doubao, SOURCE_DOUBAO),
        (Ime::Shuangpin, SOURCE_SHUANGPIN),
    ]
    .into_iter()
    .filter(|(_, id)| enabled.is_empty() || enabled.iter().any(|e| e == id))
    .map(|(ime, _)| ime)
    .collect()
}

/// The input source identifier of an input method.
pub(super) fn source_id(ime: Ime) -> Option<&'static str> {
    if !ime.is_enabled() {
        return None;
    }
    match ime {
        Ime::Us => Some(SOURCE_US),
        Ime::Doubao => Some(SOURCE_DOUBAO),
        Ime::Shuangpin => Some(SOURCE_SHUANGPIN),
        Ime::Mingkuai => mingkuai_wire::enabled().then_some(SOURCE_MINGKUAI),
        Ime::Pinyin | Ime::Other => None,
    }
}

/// The input method of an input source.
pub(super) fn ime_of(source: &str) -> Ime {
    let ime = known_ime_of(source);
    if ime.is_enabled() { ime } else { Ime::Other }
}

fn known_ime_of(source: &str) -> Ime {
    match source {
        SOURCE_US => Ime::Us,
        SOURCE_DOUBAO => Ime::Doubao,
        SOURCE_SHUANGPIN => Ime::Shuangpin,
        SOURCE_MINGKUAI if mingkuai_wire::enabled() => Ime::Mingkuai,
        _ => Ime::Other,
    }
}

fn data_dir() -> Result<PathBuf> {
    let config = config_path().context("no configuration directory")?;
    Ok(config
        .parent()
        .context("no parent directory")?
        .to_path_buf())
}

/// The Chinese input method used last. Doubao when none is recorded.
pub(super) fn last_chinese() -> Ime {
    data_dir()
        .ok()
        .and_then(|d| fs::read_to_string(d.join("last_chinese")).ok())
        .and_then(|s| Ime::parse(s.trim()))
        .filter(|i| matches!(i, Ime::Doubao | Ime::Shuangpin | Ime::Mingkuai))
        .unwrap_or(Ime::Doubao)
}

/// Records the Chinese input method used last.
pub(super) fn remember_chinese(ime: Ime) {
    if let Ok(dir) = data_dir() {
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join("last_chinese"), ime.name());
    }
}
