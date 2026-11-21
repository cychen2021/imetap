//! What the HUD and the tray icon show: which input method is active and, if known, whether it is
//! in Chinese or English mode.

use std::fmt;

/// The input methods the tool knows about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ime {
    /// The plain US keyboard layout.
    Us,
    /// Microsoft Pinyin (Windows).
    Pinyin,
    /// Doubao IME (both platforms).
    Doubao,
    /// Apple Shuangpin (macOS).
    Shuangpin,
    /// Anything else (another layout or IME).
    Other,
}

impl Ime {
    /// The English name, used in messages.
    pub fn name(self) -> &'static str {
        match self {
            Ime::Us => "US keyboard",
            Ime::Pinyin => "Microsoft Pinyin",
            Ime::Doubao => "Doubao",
            Ime::Shuangpin => "Shuangpin",
            Ime::Other => "Other",
        }
    }

    /// The name shown on the HUD: the Chinese input methods under their Chinese names.
    pub fn display_name(self) -> &'static str {
        match self {
            Ime::Pinyin => "微软拼音",
            Ime::Doubao => "豆包",
            Ime::Shuangpin => "双拼",
            Ime::Us | Ime::Other => self.name(),
        }
    }

    /// Parses a command-line name (`us`, `pinyin`, `doubao`, `shuangpin`).
    pub fn parse(name: &str) -> Option<Ime> {
        match name.to_ascii_lowercase().as_str() {
            "us" => Some(Ime::Us),
            "pinyin" => Some(Ime::Pinyin),
            "doubao" => Some(Ime::Doubao),
            "shuangpin" => Some(Ime::Shuangpin),
            _ => None,
        }
    }
}

/// The observable state of the foreground window's input method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub ime: Ime,
    /// `Some(true)` for Chinese mode, `Some(false)` for English mode, `None` when it cannot be
    /// told (the US layout has no such mode, and some input methods do not expose it).
    pub chinese: Option<bool>,
}

impl Snapshot {
    pub fn new(ime: Ime, chinese: Option<bool>) -> Self {
        Self { ime, chinese }
    }

    /// The big text on the HUD and in the tray icon.
    pub fn glyph(&self) -> &'static str {
        match (self.ime, self.chinese) {
            (Ime::Us, _) => "EN",
            (Ime::Other, None) => "?",
            (_, Some(true)) => "中",
            (_, Some(false)) => "EN",
            (_, None) => "中",
        }
    }

    /// The small text under the glyph.
    pub fn caption(&self) -> String {
        match (self.ime, self.chinese) {
            (Ime::Us | Ime::Other, _) => self.ime.display_name().to_string(),
            (ime, Some(true)) => format!("{} · 中文", ime.display_name()),
            (ime, Some(false)) => format!("{} · English", ime.display_name()),
            (ime, None) => ime.display_name().to_string(),
        }
    }
}

impl fmt::Display for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.glyph(), self.caption())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_and_caption_follow_the_mode() {
        let s = Snapshot::new(Ime::Pinyin, Some(true));
        assert_eq!((s.glyph(), s.caption().as_str()), ("中", "微软拼音 · 中文"));
        let s = Snapshot::new(Ime::Doubao, Some(false));
        assert_eq!((s.glyph(), s.caption().as_str()), ("EN", "豆包 · English"));
    }

    #[test]
    fn the_us_layout_has_no_mode() {
        let s = Snapshot::new(Ime::Us, None);
        assert_eq!((s.glyph(), s.caption().as_str()), ("EN", "US keyboard"));
    }

    #[test]
    fn an_unknown_mode_falls_back_to_the_name() {
        let s = Snapshot::new(Ime::Shuangpin, None);
        assert_eq!((s.glyph(), s.caption().as_str()), ("中", "双拼"));
    }

    #[test]
    fn names_parse_case_insensitively() {
        assert_eq!(Ime::parse("US"), Some(Ime::Us));
        assert_eq!(Ime::parse("Doubao"), Some(Ime::Doubao));
        assert_eq!(Ime::parse("nope"), None);
    }
}
