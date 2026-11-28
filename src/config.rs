//! `config.toml`: where it lives and what it may contain.
//!
//! The file is looked up in `$XDG_CONFIG_HOME/imetap/` when that variable is set, and otherwise in
//! the platform's default configuration directory (`%APPDATA%` on Windows,
//! `~/Library/Application Support` on macOS, `~/.config` elsewhere). A missing file means all
//! defaults. Every key is optional.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::fade::Fade;
use crate::reminder::ReminderMode;
use crate::state::{Color, HudColors, HudStyle, Ime, Snapshot, color_for_name, label_for_name};
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
    /// How long to wait after a double tap for a third tap, in milliseconds. 0 turns triple taps
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

/// `[hotkeys]`: one shortcut per input method. An empty string disables it.
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
    /// On a window switch, wait until focus is in a text field before showing it (Windows only).
    pub wait_for_text_field: bool,
    /// How long it stays fully visible, in milliseconds.
    pub duration_ms: u64,
    /// How long it takes to fade in, in milliseconds. 0 for none.
    pub fade_in_ms: u64,
    /// How long it takes to fade out, in milliseconds. 0 for none.
    pub fade_out_ms: u64,
    /// 0.0 (invisible) to 1.0 (opaque).
    pub opacity: f32,
    /// Background colour by mode.
    pub colors: ModeColors,
    /// Stripe colour by input method.
    pub ime_colors: ImeColors,
    /// The big label on the stripe, by input method.
    pub ime_labels: ImeLabels,
    /// Stripe colour and label of Mingkuai. Not a key of `[hud]`: `Config::parse` copies them
    /// from the hidden `[mingkuai-integrate]`.
    #[serde(skip)]
    pub mingkuai: (Color, String),
}

/// `[hud.colors]`: the background for Chinese and for English mode, by colour name.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ModeColors {
    pub chinese: Color,
    pub english: Color,
}

impl Default for ModeColors {
    fn default() -> Self {
        Self {
            chinese: Color::Red,
            english: Color::Blue,
        }
    }
}

/// `[hud.ime_colors]`: the stripe colour of each input method, by colour name. For an input method
/// without a Chinese/English mode (`other`), it is the background as well.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ImeColors {
    pub us: Color,
    /// Microsoft Pinyin on Windows.
    pub pinyin: Color,
    pub doubao: Color,
    /// Shuangpin on macOS.
    pub shuangpin: Color,
    /// Any input method imetap does not know.
    pub other: Color,
}

impl Default for ImeColors {
    fn default() -> Self {
        Self {
            us: Color::Grey,
            pinyin: Color::Green,
            doubao: Color::Orange,
            shuangpin: Color::Teal,
            other: Color::Purple,
        }
    }
}

/// `[hud.ime_labels]`: the big label on the stripe, one or two characters per input method.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ImeLabels {
    pub us: String,
    pub pinyin: String,
    pub doubao: String,
    pub shuangpin: String,
    pub other: String,
}

impl Default for ImeLabels {
    fn default() -> Self {
        Self {
            us: "US".into(),
            pinyin: "微".into(),
            doubao: "豆".into(),
            shuangpin: "双".into(),
            other: "其".into(),
        }
    }
}

impl HudConfig {
    /// The brightness curve of one appearance.
    pub fn fade(&self) -> Fade {
        Fade::new(self.fade_in_ms, self.duration_ms, self.fade_out_ms)
    }

    /// What the HUD draws for a state. The background follows the mode. An input method without a
    /// mode takes its own colour for the background.
    pub fn style(&self, snapshot: Snapshot) -> HudStyle {
        let (stripe, label) = match (snapshot.ime, snapshot.other_name()) {
            // An unknown input method that the system named: label and colour come from the name.
            (Ime::Other, Some(name)) => (color_for_name(&name), label_for_name(&name)),
            (Ime::Us, _) => (self.ime_colors.us, self.ime_labels.us.clone()),
            (Ime::Pinyin, _) => (self.ime_colors.pinyin, self.ime_labels.pinyin.clone()),
            (Ime::Doubao, _) => (self.ime_colors.doubao, self.ime_labels.doubao.clone()),
            (Ime::Shuangpin, _) => (self.ime_colors.shuangpin, self.ime_labels.shuangpin.clone()),
            (Ime::Mingkuai, _) => (self.mingkuai.0, self.mingkuai.1.clone()),
            (Ime::Other, None) => (self.ime_colors.other, self.ime_labels.other.clone()),
        };
        let background = match snapshot.glyph() {
            "中" => self.colors.chinese,
            "EN" => self.colors.english,
            _ => stripe,
        };
        HudStyle {
            colors: HudColors::new(background, stripe),
            label,
        }
    }
}

impl Default for HudConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_window_switch: true,
            wait_for_text_field: true,
            duration_ms: 1500,
            fade_in_ms: 350,
            fade_out_ms: 250,
            opacity: 0.85,
            colors: ModeColors::default(),
            ime_colors: ImeColors::default(),
            ime_labels: ImeLabels::default(),
            mingkuai: (Color::Teal, "明".into()),
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

/// `[tray]`: the tray icon on Windows and the menu bar icon on macOS.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TrayConfig {
    /// Whether the icon follows the language: "中" or "E". When false it always shows the logo.
    pub enable_lang_indicator: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enable_lang_indicator: true,
        }
    }
}

/// The whole file.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The input methods imetap handles: `us`, `pinyin`, `doubao`, `shuangpin`. The others count
    /// as unknown input methods. Mingkuai is not governed by it.
    pub enable: Vec<String>,
    pub tap: TapConfig,
    pub hotkeys: HotkeyConfig,
    pub hud: HudConfig,
    pub tray: TrayConfig,
    pub reminder: ReminderConfig,
    /// The hidden Mingkuai support. It is not in `config.example.toml`.
    #[serde(rename = "mingkuai-integrate")]
    pub mingkuai: MingkuaiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enable: ["us", "pinyin", "doubao", "shuangpin"]
                .map(String::from)
                .into(),
            tap: TapConfig::default(),
            hotkeys: HotkeyConfig::default(),
            hud: HudConfig::default(),
            tray: TrayConfig::default(),
            reminder: ReminderConfig::default(),
            mingkuai: MingkuaiConfig::default(),
        }
    }
}

/// `[mingkuai-integrate]`: the hidden Mingkuai support. Off by default.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MingkuaiConfig {
    /// Whether imetap knows Mingkuai. When false, Mingkuai counts as an unknown input method.
    pub enable: bool,
    /// The hotkey that switches to Mingkuai. Empty disables it.
    pub hotkey: String,
    /// The colour of its stripe on the HUD.
    pub color: Color,
    /// Its big label on the HUD, one or two characters.
    pub label: String,
}

impl Default for MingkuaiConfig {
    fn default() -> Self {
        Self {
            enable: false,
            hotkey: String::new(),
            color: Color::Teal,
            label: "明".into(),
        }
    }
}

impl Config {
    /// Parses the text of a `config.toml` and checks the values.
    pub fn parse(text: &str) -> Result<Config> {
        let mut config: Config = toml::from_str(text)?;
        config.hud.mingkuai = (config.mingkuai.color, config.mingkuai.label.clone());
        config.validate()?;
        Ok(config)
    }

    /// Reads the file. A missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Config> {
        match fs::read_to_string(path) {
            Ok(text) => Config::parse(&text).with_context(|| format!("invalid {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    /// The input methods named by `enable`.
    pub fn enabled_imes(&self) -> Result<Vec<Ime>> {
        self.enable
            .iter()
            .map(|name| {
                Ime::parse(name)
                    .filter(|&ime| ime.is_governed())
                    .ok_or_else(|| {
                        anyhow!("enable: unknown input method `{name}` (use us, pinyin, doubao, shuangpin)")
                    })
            })
            .collect()
    }

    /// The shortcut bound to an input method, `None` when disabled.
    pub fn shortcut(&self, ime: Ime) -> Result<Option<Shortcut>> {
        if !self.enabled_imes()?.contains(&ime) && ime.is_governed() {
            return Ok(None);
        }
        let text = match ime {
            Ime::Us => &self.hotkeys.us,
            Ime::Pinyin | Ime::Shuangpin => &self.hotkeys.chinese,
            Ime::Doubao => &self.hotkeys.doubao,
            Ime::Mingkuai if self.mingkuai.enable => &self.mingkuai.hotkey,
            Ime::Mingkuai | Ime::Other => return Ok(None),
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
        if self.hud.fade_in_ms > 2000 || self.hud.fade_out_ms > 2000 {
            bail!("hud.fade_in_ms and hud.fade_out_ms must be at most 2000");
        }
        let labels = &self.hud.ime_labels;
        for (name, label) in [
            ("us", &labels.us),
            ("pinyin", &labels.pinyin),
            ("doubao", &labels.doubao),
            ("shuangpin", &labels.shuangpin),
            ("other", &labels.other),
        ] {
            if !(1..=2).contains(&label.chars().count()) {
                bail!("hud.ime_labels.{name} must be one or two characters");
            }
        }
        if !(1..=2).contains(&self.mingkuai.label.chars().count()) {
            bail!("mingkuai-integrate.label must be one or two characters");
        }
        if self.reminder.idle_secs == 0 || self.reminder.dwell_secs == 0 {
            bail!("reminder.idle_secs and reminder.dwell_secs must be at least 1");
        }
        self.enabled_imes()?;
        for ime in [Ime::Us, Ime::Pinyin, Ime::Doubao, Ime::Mingkuai] {
            self.shortcut(ime)?;
        }
        Ok(())
    }
}

/// A commented `config.toml` with every default spelled out (`config.example.toml`). Written by
/// `imetap init`.
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
    fn the_fade_follows_the_config() {
        let hud = Config::parse("[hud]\nduration_ms = 2000\nfade_in_ms = 100\nfade_out_ms = 50\n")
            .unwrap()
            .hud;
        assert_eq!(hud.fade(), Fade::new(100, 2000, 50));
        assert_eq!(HudConfig::default().fade(), Fade::new(350, 1500, 250));
        assert!(Config::parse("[hud]\nfade_in_ms = 0\n").is_ok());
        assert!(Config::parse("[hud]\nfade_in_ms = 5000\n").is_err());
        assert!(Config::parse("[hud]\nfade_out_ms = 5000\n").is_err());
    }

    #[test]
    fn colours_are_chosen_by_name() {
        let c = Config::parse(
            "[hud.colors]\nchinese = \"purple\"\n[hud.ime_colors]\ndoubao = \"pink\"\n",
        )
        .unwrap();
        assert_eq!(c.hud.colors.chinese, Color::Purple);
        assert_eq!(c.hud.colors.english, Color::Blue);
        assert_eq!(c.hud.ime_colors.doubao, Color::Pink);
        assert!(Config::parse("[hud.colors]\nchinese = \"#ff0000\"\n").is_err());
        assert!(Config::parse("[hud.colors]\nchinese = \"chartreuse\"\n").is_err());
    }

    #[test]
    fn labels_are_one_or_two_characters() {
        assert!(Config::parse("[hud.ime_labels]\ndoubao = \"豆包\"\n").is_ok());
        assert!(Config::parse("[hud.ime_labels]\ndoubao = \"\"\n").is_err());
        assert!(Config::parse("[hud.ime_labels]\ndoubao = \"豆包输\"\n").is_err());
    }

    #[test]
    fn the_background_follows_the_mode_and_the_stripe_the_input_method() {
        let hud = HudConfig::default();
        let zh = hud.style(Snapshot::new(Ime::Doubao, Some(true)));
        assert_eq!(zh.colors, HudColors::new(Color::Red, Color::Orange));
        assert_eq!(zh.label, "豆");
        let en = hud.style(Snapshot::new(Ime::Pinyin, Some(false)));
        assert_eq!(en.colors, HudColors::new(Color::Blue, Color::Green));
        let us = hud.style(Snapshot::new(Ime::Us, None));
        assert_eq!(us.colors, HudColors::new(Color::Blue, Color::Grey));
        assert_eq!(us.label, "US");
        // An unknown input method that the system named gets a label and colour from its name.
        let named = hud.style(Snapshot::new(Ime::Other, None).with_other_name("Hiragana"));
        assert_eq!(named.label, "HI");
        let color = color_for_name("Hiragana");
        assert_eq!(named.colors, HudColors::new(color, color));
        // Without a mode, the input method's own colour is the background.
        let other = hud.style(Snapshot::new(Ime::Other, None));
        assert_eq!(other.colors, HudColors::new(Color::Purple, Color::Purple));
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
    fn enable_picks_the_input_methods() {
        let c = Config::default();
        assert_eq!(c.enabled_imes().unwrap().len(), 4);
        let c = Config::parse("enable = [\"us\", \"pinyin\"]\n").unwrap();
        assert!(c.shortcut(Ime::Us).unwrap().is_some());
        assert!(c.shortcut(Ime::Pinyin).unwrap().is_some());
        assert_eq!(c.shortcut(Ime::Doubao).unwrap(), None);
        assert!(Config::parse("enable = [\"nope\"]\n").is_err());
        assert!(Config::parse("enable = [\"mingkuai\"]\n").is_err());
    }

    #[test]
    fn a_bad_hotkey_fails_the_whole_file() {
        assert!(Config::parse("[hotkeys]\ndoubao = \"D\"\n").is_err());
    }

    #[test]
    fn the_mingkuai_support_is_hidden_and_off_by_default() {
        let c = Config::default();
        assert!(!c.mingkuai.enable);
        assert!(!DEFAULT_TOML.contains("mingkuai"));
        assert_eq!(c.shortcut(Ime::Mingkuai).unwrap(), None);
        let text = "[mingkuai-integrate]\nenable = true\nhotkey = \"Ctrl+Shift+M\"\ncolor = \"pink\"\nlabel = \"明快\"\n";
        let c = Config::parse(text).unwrap();
        assert!(c.mingkuai.enable);
        assert!(c.shortcut(Ime::Mingkuai).unwrap().is_some());
        let style = c.hud.style(Snapshot::new(Ime::Mingkuai, Some(true)));
        assert_eq!(style.label, "明快");
        assert!(Config::parse("[mingkuai-integrate]\nlabel = \"\"\n").is_err());
        assert!(Config::parse("[mingkuai-integrate]\nunknown = 1\n").is_err());
    }

    #[test]
    fn the_tray_language_indicator_is_on_by_default() {
        assert!(Config::default().tray.enable_lang_indicator);
        assert!(
            !Config::parse("[tray]\nenable_lang_indicator = false\n")
                .unwrap()
                .tray
                .enable_lang_indicator
        );
        assert!(Config::parse("[tray]\nunknown = 1\n").is_err());
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
