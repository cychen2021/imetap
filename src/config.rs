//! `config.toml`: where it lives and what it may contain.
//!
//! The file is looked up in `$XDG_CONFIG_HOME/imetap/` when that variable is set, and otherwise in
//! the platform's default configuration directory (`%APPDATA%` on Windows,
//! `~/Library/Application Support` on macOS, `~/.config` elsewhere). A missing file means all
//! defaults; every key is optional.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::reminder::ReminderMode;
use crate::state::Ime;
use crate::tap::TapDetector;

/// Modifier keys of a [`Shortcut`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Windows key, or Command on macOS.
    pub meta: bool,
}

/// A global hotkey such as `Ctrl+Shift+S`: modifiers plus one letter, digit or function key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub mods: Modifiers,
    pub key: Key,
}

/// The key part of a [`Shortcut`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// `A` to `Z`, stored upper case.
    Letter(char),
    /// `0` to `9`.
    Digit(char),
    /// `F1` to `F12`.
    Function(u8),
}

impl Shortcut {
    /// Parses `Ctrl+Shift+S`, `cmd+alt+F5` and the like (case-insensitive, `+` separated).
    pub fn parse(text: &str) -> Result<Shortcut> {
        let mut mods = Modifiers::default();
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" | "option" | "opt" => mods.alt = true,
                "win" | "meta" | "super" | "cmd" | "command" => mods.meta = true,
                other => {
                    if key.replace(parse_key(other)?).is_some() {
                        bail!("`{text}` names more than one key");
                    }
                }
            }
        }
        let key = key.with_context(|| format!("`{text}` has no key"))?;
        if mods == Modifiers::default() {
            bail!("`{text}` needs at least one modifier");
        }
        Ok(Shortcut { mods, key })
    }
}

fn parse_key(name: &str) -> Result<Key> {
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Ok(Key::Letter(c.to_ascii_uppercase())),
        (Some(c), None) if c.is_ascii_digit() => Ok(Key::Digit(c)),
        (Some('f'), Some(_)) => match name[1..].parse::<u8>() {
            Ok(n @ 1..=12) => Ok(Key::Function(n)),
            _ => bail!("unknown key `{name}`"),
        },
        _ => bail!("unknown key `{name}`"),
    }
}

/// `[tap]`: the Ctrl double-tap that toggles Chinese/English, and the triple-tap that shows the HUD.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TapConfig {
    pub enabled: bool,
    /// Longest time between the two taps, in milliseconds.
    pub gap_ms: u64,
    /// Longest time Ctrl may be held for a press to still be a tap, in milliseconds.
    pub hold_ms: u64,
    /// How long to wait after a double tap for a third tap, in milliseconds; 0 turns triple taps
    /// off, so double taps act at once.
    pub triple_ms: u64,
    /// Hold back keys typed while a double tap is being applied, and replay them afterwards, so
    /// that they reach the application in the new mode (Windows only).
    pub hold_input: bool,
}

impl Default for TapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gap_ms: TapDetector::DEFAULT_GAP_MS,
            hold_ms: TapDetector::DEFAULT_HOLD_MS,
            triple_ms: TapDetector::DEFAULT_TRIPLE_MS,
            hold_input: true,
        }
    }
}

/// `[hotkeys]`: one shortcut per input method; an empty string disables it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HotkeyConfig {
    pub us: String,
    /// Microsoft Pinyin on Windows, Shuangpin on macOS.
    pub chinese: String,
    pub doubao: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            us: "Ctrl+Shift+S".into(),
            chinese: "Ctrl+Shift+F".into(),
            doubao: "Ctrl+Shift+D".into(),
        }
    }
}

/// `[hud]`: the overlay shown on every change.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HudConfig {
    pub enabled: bool,
    /// Also show it when switching to a window whose input method or mode differs.
    pub on_window_switch: bool,
    /// How long it stays fully visible, in milliseconds.
    pub duration_ms: u64,
    /// 0.0 (invisible) to 1.0 (opaque).
    pub opacity: f32,
}

impl Default for HudConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_window_switch: true,
            duration_ms: 900,
            opacity: 0.85,
        }
    }
}

/// `[reminder]`: show the HUD again when the input method may have been forgotten.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ReminderConfig {
    pub enabled: bool,
    pub mode: ReminderMode,
    /// How long without key, mouse or focus-window activity counts as idle, in seconds.
    pub idle_secs: u64,
    /// In `resume-and-switch` mode, how long a window must have been used, without the HUD
    /// appearing, for leaving it to show the HUD, in seconds.
    pub dwell_secs: u64,
}

impl Default for ReminderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ReminderMode::Resume,
            idle_secs: 300,
            dwell_secs: 300,
        }
    }
}

/// The whole file.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub tap: TapConfig,
    pub hotkeys: HotkeyConfig,
    pub hud: HudConfig,
    pub reminder: ReminderConfig,
}

impl Config {
    /// Parses the text of a `config.toml` and checks the values.
    pub fn parse(text: &str) -> Result<Config> {
        let config: Config = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Reads the file; a missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Config> {
        match fs::read_to_string(path) {
            Ok(text) => Config::parse(&text).with_context(|| format!("invalid {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    /// The shortcut bound to an input method, `None` when disabled.
    pub fn shortcut(&self, ime: Ime) -> Result<Option<Shortcut>> {
        let text = match ime {
            Ime::Us => &self.hotkeys.us,
            Ime::Pinyin | Ime::Shuangpin => &self.hotkeys.chinese,
            Ime::Doubao => &self.hotkeys.doubao,
            Ime::Other => return Ok(None),
        };
        if text.trim().is_empty() {
            return Ok(None);
        }
        Shortcut::parse(text)
            .map(Some)
            .with_context(|| format!("bad hotkey for {}", ime.name()))
    }

    fn validate(&self) -> Result<()> {
        if !(0.05..=1.0).contains(&self.hud.opacity) {
            bail!("hud.opacity must be between 0.05 and 1.0");
        }
        if self.reminder.idle_secs == 0 || self.reminder.dwell_secs == 0 {
            bail!("reminder.idle_secs and reminder.dwell_secs must be at least 1");
        }
        for ime in [Ime::Us, Ime::Pinyin, Ime::Doubao] {
            self.shortcut(ime)?;
        }
        Ok(())
    }
}

/// A commented `config.toml` with every default spelled out (`config.example.toml`); written by
/// `imetap config init`.
pub const DEFAULT_TOML: &str = include_str!("../config.example.toml");

/// Where `config.toml` is looked up.
pub fn config_path() -> Option<PathBuf> {
    let xdg = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    Some(
        xdg.or_else(default_config_dir)?
            .join("imetap")
            .join("config.toml"),
    )
}

#[cfg(windows)]
fn default_config_dir() -> Option<PathBuf> {
    env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn default_config_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Application Support"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn default_config_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_gives_the_defaults() {
        let c = Config::parse("").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.tap.gap_ms, 500);
        assert_eq!(c.hotkeys.us, "Ctrl+Shift+S");
    }

    #[test]
    fn keys_override_only_what_they_name() {
        let c = Config::parse("[tap]\ngap_ms = 700\n[hud]\nenabled = false\n").unwrap();
        assert_eq!(c.tap.gap_ms, 700);
        assert_eq!(c.tap.hold_ms, 300);
        assert!(!c.hud.enabled);
        assert!(c.hud.on_window_switch);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(Config::parse("[tap]\ngapp_ms = 700\n").is_err());
        assert!(Config::parse("[nope]\n").is_err());
    }

    #[test]
    fn opacity_is_range_checked() {
        assert!(Config::parse("[hud]\nopacity = 0\n").is_err());
        assert!(Config::parse("[hud]\nopacity = 1.5\n").is_err());
        assert!(Config::parse("[hud]\nopacity = 0.5\n").is_ok());
    }

    #[test]
    fn the_reminder_is_off_by_default_and_configurable() {
        assert!(!Config::default().reminder.enabled);
        let c = Config::parse(
            "[reminder]\nenabled = true\nmode = \"resume-and-switch\"\ndwell_secs = 120\n",
        )
        .unwrap();
        assert!(c.reminder.enabled);
        assert_eq!(c.reminder.mode, ReminderMode::ResumeAndSwitch);
        assert_eq!(c.reminder.dwell_secs, 120);
        assert_eq!(c.reminder.idle_secs, 300);
        assert!(Config::parse("[reminder]\nidle_secs = 0\n").is_err());
    }

    #[test]
    fn shortcuts_parse() {
        let s = Shortcut::parse("Ctrl+Shift+S").unwrap();
        assert_eq!(s.key, Key::Letter('S'));
        assert!(s.mods.ctrl && s.mods.shift && !s.mods.alt && !s.mods.meta);
        let s = Shortcut::parse("cmd + alt + f5").unwrap();
        assert_eq!(s.key, Key::Function(5));
        assert!(s.mods.meta && s.mods.alt);
        assert_eq!(Shortcut::parse("Ctrl+7").unwrap().key, Key::Digit('7'));
    }

    #[test]
    fn bad_shortcuts_are_rejected() {
        for bad in [
            "S",
            "Ctrl+",
            "Ctrl+Shift",
            "Ctrl+S+D",
            "Ctrl+F13",
            "Ctrl+Space",
        ] {
            assert!(Shortcut::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn an_empty_shortcut_disables_the_hotkey() {
        let c = Config::parse("[hotkeys]\nus = \"\"\n").unwrap();
        assert_eq!(c.shortcut(Ime::Us).unwrap(), None);
        assert!(c.shortcut(Ime::Doubao).unwrap().is_some());
    }

    #[test]
    fn a_bad_hotkey_fails_the_whole_file() {
        assert!(Config::parse("[hotkeys]\ndoubao = \"D\"\n").is_err());
    }

    #[test]
    fn the_template_spells_out_the_defaults() {
        assert_eq!(Config::parse(DEFAULT_TOML).unwrap(), Config::default());
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let c = Config::load(Path::new("definitely/not/here/config.toml")).unwrap();
        assert_eq!(c, Config::default());
    }
}
