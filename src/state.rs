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

/// An sRGB colour.
pub type Rgb = [u8; 3];

/// The colours of the HUD for one state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudColors {
    pub background: Rgb,
    pub glyph: Rgb,
    pub caption: Rgb,
}

impl HudColors {
    /// Chinese: deep red, like the Windows tray icon.
    pub const CHINESE: HudColors = HudColors {
        background: [0xA8, 0x32, 0x2B],
        glyph: [0xFF, 0xFF, 0xFF],
        // White at 78% over the background.
        caption: [0xEC, 0xD2, 0xD0],
    };
    /// English, including the US layout: deep blue, like the Windows tray icon.
    pub const ENGLISH: HudColors = HudColors {
        background: [0x1F, 0x5A, 0xA6],
        glyph: [0xFF, 0xFF, 0xFF],
        // White at 78% over the background.
        caption: [0xCE, 0xDB, 0xEB],
    };
    /// An input method imetap does not know: neutral dark grey.
    pub const UNKNOWN: HudColors = HudColors {
        background: [0x20, 0x21, 0x24],
        glyph: [0xFF, 0xFF, 0xFF],
        caption: [0xBE, 0xC0, 0xC4],
    };
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

    /// The HUD's colours, which follow the glyph: red for "中", blue for "EN" (the US layout
    /// included), grey for "?".
    pub fn hud_colors(&self) -> HudColors {
        match self.glyph() {
            "中" => HudColors::CHINESE,
            "EN" => HudColors::ENGLISH,
            _ => HudColors::UNKNOWN,
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

/// Decides whether a window switch (or any unprompted change) deserves a HUD when only text fields
/// count. `baseline` is the state the last time focus was in a text field. Outside a text field
/// nothing shows and the baseline stays, so leaving a Chinese text box for a window without one and
/// coming back is no change. Inside one, the HUD is due when the state differs from the baseline,
/// which is then updated.
pub fn text_field_change(
    baseline: &mut Option<Snapshot>,
    now: Snapshot,
    in_text_field: bool,
) -> bool {
    if !in_text_field {
        return false;
    }
    let due = baseline.is_some_and(|before| before != now);
    *baseline = Some(now);
    due
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_follow_the_language() {
        assert_eq!(
            Snapshot::new(Ime::Doubao, Some(true)).hud_colors(),
            HudColors::CHINESE
        );
        assert_eq!(
            Snapshot::new(Ime::Shuangpin, None).hud_colors(),
            HudColors::CHINESE
        );
        assert_eq!(
            Snapshot::new(Ime::Pinyin, Some(false)).hud_colors(),
            HudColors::ENGLISH
        );
        assert_eq!(
            Snapshot::new(Ime::Us, None).hud_colors(),
            HudColors::ENGLISH
        );
        assert_eq!(
            Snapshot::new(Ime::Other, None).hud_colors(),
            HudColors::UNKNOWN
        );
    }

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

    #[test]
    fn only_text_fields_count_for_window_switch_huds() {
        let zh = Snapshot::new(Ime::Doubao, Some(true));
        let en = Snapshot::new(Ime::Us, None);
        let mut baseline = None;
        // The first reading only sets the baseline.
        assert!(!text_field_change(&mut baseline, zh, true));
        // A Chinese text box, then a window without a text field, then the Chinese text box again.
        assert!(!text_field_change(&mut baseline, en, false));
        assert!(!text_field_change(&mut baseline, zh, true));
        // Another text field with a different state does show.
        assert!(text_field_change(&mut baseline, en, true));
        // And back.
        assert!(text_field_change(&mut baseline, zh, true));
        assert!(!text_field_change(&mut baseline, zh, true));
    }
}
